//#![feature(let_chains)]
use std::io::Write;
#[forbid(unsafe_code)]
pub mod common;
use crate::common::{Event, Packet, MessageType, PktSource};

use smol::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use std::net::{TcpListener, TcpStream};
use smol::channel::{Receiver, Sender, bounded};
use std::error::Error;
use std::net::SocketAddr;
use std::collections::HashMap;
use async_dup::Arc;
use postcard::{to_allocvec};
use smol::Async;

async fn dispatch(receiver: Receiver<Event>) -> Result<(), ()> {
    //active clients
    let mut map = HashMap::<SocketAddr, Arc<Async<TcpStream>>>::new();

    while let Ok(event) = receiver.recv().await {
        let packet = match event {
            Event::Join(addr, stream) => {
                map.insert(addr, stream);
                Packet {
                    src: PktSource::SERVER,
                    message_type: MessageType::STRING,
                    data: Vec::from(format!("{} has joined", addr)),
                }
            }
            Event::Leave(addr) => {
                map.remove(&addr);
                Packet {
                    src: PktSource::SERVER,
                    message_type: MessageType::STRING,
                    data: Vec::from(format!("{} has left", addr))
                }
            }
            Event::Message(addr, mut packet) => {
                //clients should not be able to spoof the source
                packet.src = PktSource::CLIENT(addr);
                packet
            }
        };
        println!("{}", String::from_utf8(packet.data.clone()).unwrap());
        //stdout().flush().unwrap();
        let output = to_allocvec(&packet).unwrap();

        for stream in map.values_mut() {
            // Ignore errors because the client might disconnect at any point.
            stream.write_all(output.as_slice()).await.ok();
        }
    }

    Ok(())
}

async fn read_messages(sender: Sender<Event>, client: Arc<Async<TcpStream>>) -> Result<(), Box<dyn Error>> {
    let addr = client.get_ref().peer_addr().unwrap();
    let mut reader = BufReader::new(client);

    'a : loop {
        let consumed;
        match reader.fill_buf().await {
            Ok(bytes_read) => {
                if bytes_read.len() == 0 {
                    //nothing read... is the client even connected?
                    match reader.read(&mut [0u8; 0]).await {
                        Ok(0) => {
                            // The client is disconnected
                            return Ok(())
                        }
                        _ => {}
                    }
                    continue 'a;
                }
                //todo: client can send bad data and crash server
                let packet: Packet = postcard::from_bytes(bytes_read).unwrap();
                sender.send(Event::Message(addr, packet)).await.ok();
                consumed = bytes_read.len();
            }
            Err(_) => { //todo: when is this branch taken?
                return Ok(())
            }
        }
        reader.consume(consumed);
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    smol::block_on(async {
        let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 6000))?;
        println!("Listening on: {}", listener.get_ref().local_addr()?);

        let (dispatcher_channel_sender, dispatcher_channel_receiver) = bounded(100);

        smol::spawn(dispatch(dispatcher_channel_receiver)).detach();

        loop {
            let (client_stream, addr) = listener.accept().await?;
            let client = Arc::new(client_stream);
            let sender = dispatcher_channel_sender.clone();

            smol::spawn(async move {
                sender.send(Event::Join(addr, client.clone())).await.ok();

                read_messages(sender.clone(), client).await.ok();

                sender.send(Event::Leave(addr)).await.ok();
            }).detach();
        }
    })
}
//#![feature(let_chains)]
pub mod common;
use crate::common::{Event, Packet, MessageType};

use smol::io::{AsyncReadExt, AsyncWriteExt, BufReader};
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
                    message_type: MessageType::STRING,
                    data: Vec::from(format!("{} has joined", addr)),
                }
            }
            Event::Leave(addr) => {
                map.remove(&addr);
                Packet {
                    message_type: MessageType::STRING,
                    data: Vec::from(format!("{} has left", addr))
                }
            }
            Event::Message(_addr, packet) => {
                packet
            }
        };
        println!("{}", String::from_utf8(packet.data.clone()).unwrap());
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
    let mut buf = Vec::new();

    'a : loop {
        match reader.read_to_end(&mut buf).await {
            Ok(bytes_read) => {
                if bytes_read == 0 { //nothing read
                    continue 'a;
                }
                //todo: client can send bad data and crash server
                let packet = postcard::from_bytes(buf.as_slice()).unwrap();
                sender.send(Event::Message(addr, packet)).await.ok();
            }
            Err(_) => {
                //sender.send(Event::Leave(addr)).await.ok();
                return Ok(())
            }
        }
        buf = Vec::new();
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
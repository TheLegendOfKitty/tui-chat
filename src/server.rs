//! Server

#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::nursery, clippy::cargo)]
#![allow(clippy::needless_return)]

use smol::io::{BufReader};
use std::net::{TcpListener, TcpStream};
use smol::channel::{Receiver, Sender, bounded};
use std::error::Error;
use std::net::SocketAddr;
use std::collections::HashMap;
use std::{panic};
use async_dup::Arc;
use smol::Async;

pub mod common;
use crate::common::{Event, Packet, MessageType, PktSource, read_data, ReadResult, send_with_header, ClientList, Client};

async fn dispatch(receiver: Receiver<Event>) -> Result<(), ()> {
    //active clients
    let mut map = HashMap::<SocketAddr, Arc<Async<TcpStream>>>::new();

    while let Ok(event) = receiver.recv().await {
        let packet = match event {
            Event::Join(addr, stream) => {
                map.insert(addr, stream);

                let mut clients_list = ClientList {
                    clients: Vec::new()
                };
                for key in map.keys() {
                    clients_list.clients.push(Client { addr: *key })
                }
                let packet = Packet {
                    src: PktSource::SERVER,
                    message_type: MessageType::CLIENTS,
                    data: postcard::to_allocvec(&clients_list).unwrap(),
                };
                for stream in map.values_mut() {
                    // Ignore errors because the client might disconnect at any point.
                    send_with_header(stream, packet.clone()).await.ok();
                }

                Packet {
                    src: PktSource::SERVER,
                    message_type: MessageType::STRING,
                    data: Vec::from(format!("{} has joined", addr)),
                }
            }
            Event::Leave(addr) => {
                map.remove(&addr);

                let mut clients_list = ClientList {
                    clients: Vec::new()
                };
                for key in map.keys() {
                    clients_list.clients.push(Client { addr: *key })
                }
                let packet = Packet {
                    src: PktSource::SERVER,
                    message_type: MessageType::CLIENTS,
                    data: postcard::to_allocvec(&clients_list).unwrap(),
                };
                for stream in map.values_mut() {
                    // Ignore errors because the client might disconnect at any point.
                    send_with_header(stream, packet.clone()).await.ok();
                }

                Packet {
                    src: PktSource::SERVER,
                    message_type: MessageType::STRING,
                    data: Vec::from(format!("{} has left", addr))
                }
            }
            Event::Message(addr, mut packet) => {
                //clients should not be able to spoof the source
                packet.src = PktSource::CLIENT(addr);
                //clients should not be able to send client list packets
                if packet.message_type == MessageType::CLIENTS {
                    continue;
                }
                packet
            }
        };
        if packet.message_type == MessageType::STRING {
            println!("{}", String::from_utf8(packet.data.clone()).unwrap());
        }

        for stream in map.values_mut() {
            // Ignore errors because the client might disconnect at any point.
            send_with_header(stream, packet.clone()).await.ok();
        }
    }

    Ok(())
}

async fn read_messages(sender: Sender<Event>, client: Arc<Async<TcpStream>>) -> Result<(), Box<dyn Error>> {
    let addr = client.get_ref().peer_addr().unwrap();
    let mut reader = BufReader::with_capacity(1024 * 1024 * 16 /* 16 mb */,client);

    'a : loop {
        match read_data(&mut reader).await {
            Ok(res) => {
                match res {
                    ReadResult::EMPTY => {
                        continue 'a;
                    }
                    ReadResult::DISCONNECT => {
                        return Ok(());
                    }
                    ReadResult::SUCCESS(data) => {
                        let packet : Packet = postcard::from_bytes( data.as_slice()).unwrap();
                        sender.send(Event::Message(addr, packet)).await.ok();
                    }
                }
            }
            Err(e) => {
                panic!("{}", e);
            }
        }
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    panic::set_hook(Box::new(|panic_info| {
        better_panic::Settings::auto().create_panic_handler()(panic_info);
        println!("Core dumped");
    }));
    coredump::register_panic_handler().unwrap();
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
//! Server

#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::nursery, clippy::cargo)]
use smol::io::{AsyncWriteExt, BufReader};
use std::net::{TcpListener, TcpStream};
use smol::channel::{Receiver, Sender, bounded};
use std::error::Error;
use std::net::SocketAddr;
use std::collections::HashMap;
use std::{panic};
use async_dup::Arc;
use smol::Async;

pub mod common;
use crate::common::{Event, Packet, MessageType, PktSource, read_data, ReadResult, send_with_header};

async fn dispatch(receiver: Receiver<Event>) -> Result<(), ()> {
    panic::set_hook(Box::new(|panic_info| {
        better_panic::Settings::auto().create_panic_handler()(panic_info);
    }));

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
        if packet.message_type == MessageType::STRING {
            println!("{}", String::from_utf8(packet.data.clone()).unwrap());
        }
        //let output = to_allocvec(&packet).unwrap();

        for stream in map.values_mut() {
            // Ignore errors because the client might disconnect at any point.
            send_with_header(stream, packet.clone()).await.ok();
            //stream.write_all(output.as_slice()).await.ok();
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
        /*
        let packet_length: u32;
        match reader.byte_order().read_u32::<BigEndian>().now_or_never() {
            Some(res) => {
                match res {
                    Ok(length) => {
                        packet_length = length;
                    }
                    //todo: when is this branch taken?
                    Err(_) => {
                        // Failed to read the packet length, likely due to a disconnect
                        return Ok(());
                    }
                }
            }
            None => {
                continue 'a;
            }
        }

        let mut message_data = vec![0u8; packet_length as usize];
        match reader.read_exact(&mut message_data).await {
            Ok(()) => {
                let packet: Packet = postcard::from_bytes(&message_data).unwrap();
                sender.send(Event::Message(addr, packet)).await.ok();
            }
            Err(_) => {
                // Failed to read the message data, likely due to a disconnect
                return Ok(());
            }
        }*/
        /*
        let consumed;
        match reader.fill_buf().await {
            Ok(bytes_read) => {
                if bytes_read.is_empty() {
                    //nothing read... is the client even connected?
                    if let Ok(0) = reader.read(&mut [0u8; 0]).await {
                        // The client is disconnected
                        return Ok(())
                    }
                    continue 'a;
                }
                //todo: client can send bad data and crash server
                let (header, data_bytes) = bytes_read.split_at(size_of::<u32>());

                let remaining_size = u32::from_be_bytes(header.try_into().unwrap()) - u32::try_from(data_bytes.len()).unwrap();
                if remaining_size != 0 {
                    /*
                    let raw = std::fs::read("/home/parsa/RustroverProjects/chat/target/debug/cat.jpg").unwrap();
                    let format = image::guess_format(raw.as_slice()).unwrap();
                    let packet = Packet {
                        src: PktSource::UNDEFINED,
                        message_type: MessageType::IMAGE(format),
                        data: raw,
                    };
                    let encoded = to_allocvec(&packet).unwrap();*/

                    let mut remaining_buf = Vec::new();
                    /*let matching : Vec<(&u8, &u8)> = buf.iter().zip(&encoded).filter(|&(a, b)| a != b).collect();
                    println!("{:?}", matching);*/
                    remaining_buf.resize(usize::try_from(remaining_size).unwrap(), 0);

                    let mut final_buf = Vec::from(data_bytes);

                    consumed = bytes_read.len();
                    reader.consume(consumed);

                    let res = reader.read_exact(&mut remaining_buf).await;
                    match res {
                        Ok(_) => {}
                        Err(e) => {
                            println!("{}", e);
                            return Ok(()) //the client is most likely disconnected
                        }
                    }

                    final_buf.append(&mut remaining_buf);
                    println!("{:x}", md5::compute(final_buf.as_slice()));

                    /*
                    let mut count = 0;
                    let matching : Vec<(&u8, &u8)> = final_buf.iter().zip(&encoded).filter(|&(a, b)| {
                        count += 1;
                        a != b
                    }).collect();
                    println!("{:?}", matching);
                    println!("{}", count);*/

                    let packet : Packet = postcard::from_bytes( final_buf.as_slice()).unwrap();
                    sender.send(Event::Message(addr, packet)).await.ok();
                }
                else {
                    println!("{:x}", md5::compute(data_bytes));
                    let packet : Packet = postcard::from_bytes(data_bytes).unwrap();
                    sender.send(Event::Message(addr, packet)).await.ok();
                    consumed = bytes_read.len();
                    reader.consume(consumed);
                }
            }
            Err(_) => { //todo: when is this branch taken?
                return Ok(())
            }
        }*/
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
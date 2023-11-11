#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::nursery, clippy::cargo)]

use std::io;
use std::mem::size_of;
use image::ImageFormat;
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use async_dup::Arc;
use postcard::to_allocvec;
use smol::Async;
use smol::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use crate::common::ReadResult::{*};

pub enum Event {
    Join(SocketAddr, Arc<Async<TcpStream>>),
    Leave(SocketAddr),
    Message(SocketAddr, Packet)
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[derive(Clone)]
#[serde(remote = "ImageFormat")]
#[non_exhaustive]
pub enum ImageFormatDef {
    Png,
    Jpeg,
    Gif,
    WebP,
    Pnm,
    Tiff,
    Tga,
    Dds,
    Bmp,
    Ico,
    Hdr,
    OpenExr,
    Farbfeld,
    Avif,
    Qoi,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[derive(Clone)]
pub enum MessageType {
    STRING,
    #[serde(with = "ImageFormatDef")]
    IMAGE(ImageFormat)
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[derive(Clone)]
pub enum PktSource {
    CLIENT(SocketAddr), //set by server when messages are dispatched
    SERVER, //server messages, such as join/leave
    UNDEFINED //client should set source as undefined
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[derive(Clone)]
pub struct Packet {
    pub src: PktSource,
    pub message_type: MessageType,
    pub data: Vec<u8>
}

pub enum ReadResult {
    EMPTY,
    DISCONNECT,
    SUCCESS(Vec<u8>)
}

/// This function should not panic
pub async fn send_with_header(writer: &mut Arc<Async<TcpStream>>, packet: Packet) -> io::Result<()> {
    let encoded = to_allocvec(&packet).unwrap();
    let size : u32 = u32::try_from(encoded.len() * size_of::<u8>()).unwrap();

    match writer.write(&size.to_be_bytes()).await {
        Ok(_) => {}
        Err(err) => {
            return Err(err);
        }
    };
    match writer.write(encoded.as_slice()).await {
        Ok(_) => {}
        Err(err) => {
            return Err(err);
        }
    };
    return Ok(());
}

/// This function should not panic
pub async fn read_data(reader: &mut BufReader<Arc<Async<TcpStream>>>) -> io::Result<ReadResult> {
    let consumed;
    match reader.fill_buf().await {
        Ok(bytes_read) => {
            if bytes_read.is_empty() {
                //nothing read... is the client even connected?
                if let Ok(0) = reader.read(&mut [0u8; 0]).await {
                    // The client is disconnected
                    return Ok(DISCONNECT);
                }
                return Ok(EMPTY);
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

                //todo: can client never send the second part and leave us hanging?
                let res = reader.read_exact(&mut remaining_buf).await;
                match res {
                    Ok(_) => {}
                    Err(_) => {
                        //println!("{}", e);
                        return Ok(DISCONNECT) //the client is most likely disconnected
                    }
                }

                final_buf.append(&mut remaining_buf);
                //println!("{:x}", md5::compute(final_buf.as_slice()));

                /*
                let mut count = 0;
                let matching : Vec<(&u8, &u8)> = final_buf.iter().zip(&encoded).filter(|&(a, b)| {
                    count += 1;
                    a != b
                }).collect();
                println!("{:?}", matching);
                println!("{}", count);*/

                //let packet : Packet = postcard::from_bytes( final_buf.as_slice()).unwrap();
                //sender.send(Event::Message(addr, packet)).await.ok();
                return Ok(SUCCESS(final_buf));

            }
            else {
                //println!("{:x}", md5::compute(data_bytes));
                //let packet : Packet = postcard::from_bytes(data_bytes).unwrap();
                //sender.send(Event::Message(addr, packet)).await.ok();
                let final_buf = Vec::from(data_bytes);
                consumed = bytes_read.len();
                reader.consume(consumed);
                return Ok(SUCCESS(final_buf));
            }
        }
        Err(e) => { //todo: when is this branch taken?
            return Err(e);
        }
    }
}
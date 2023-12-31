#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::nursery, clippy::cargo)]
#![allow(clippy::needless_return)]

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
pub struct ImageData {
    #[serde(with = "ImageFormatDef")]
    pub format: ImageFormat,
    pub name: String
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[derive(Clone)]
pub enum MessageType {
    STRING,
    IMAGE(ImageData),
    CLIENTS
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Client {
    pub addr: SocketAddr
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct ClientList {
    pub clients: Vec<Client>
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

/// This function should not panic or use stdout
pub async fn send_with_header(writer: &mut Arc<Async<TcpStream>>, packet: Packet) -> io::Result<()> {
    let encoded = to_allocvec(&packet).unwrap();
    //todo: no need to multiply by size_of::<u8>?
    let size : u32 = u32::try_from(encoded.len() * size_of::<u8>()).unwrap();
    let size_bytes = size.to_be_bytes();

    match writer.write(&size_bytes).await {
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

/// This function should not panic or use stdout
pub async fn read_data(reader: &mut BufReader<Arc<Async<TcpStream>>>) -> io::Result<ReadResult> {
    let consumed;
    match reader.fill_buf().await {
        Ok(bytes_read) => {
            if bytes_read.is_empty() {
                //nothing read... is the client even connected?
                if matches!(reader.read(&mut [0u8; 0]).await, Ok(0)) {
                    // The client is disconnected
                    return Ok(DISCONNECT);
                }
                return Ok(EMPTY);
            }
            //todo: client can send bad data and crash server
            let (header, data_bytes) = bytes_read.split_at(size_of::<u32>());

            let remaining = u32::from_be_bytes(header.try_into().unwrap());
            let read = u32::try_from(data_bytes.len()).unwrap();
            if remaining < read {
                let final_buf : Vec<u8> = data_bytes[..usize::try_from(remaining).unwrap()].to_vec();
                reader.consume(final_buf.len() + size_of::<u32>());
                return Ok(SUCCESS(final_buf))
            }
            let remaining_size = remaining - read;
            if remaining_size != 0 {
                let mut remaining_buf = vec![0; usize::try_from(remaining_size).unwrap()];

                let mut final_buf = Vec::from(data_bytes);

                consumed = bytes_read.len();
                reader.consume(consumed);

                //todo: can client never send the second part and leave us hanging?
                let res = reader.read_exact(&mut remaining_buf).await;
                match res {
                    Ok(_) => {}
                    Err(_) => {
                        return Ok(DISCONNECT) //the client is most likely disconnected
                    }
                }

                final_buf.append(&mut remaining_buf);

                return Ok(SUCCESS(final_buf));

            }
            else {
                let final_buf = Vec::from(data_bytes);
                consumed = bytes_read.len();
                reader.consume(consumed);
                return Ok(SUCCESS(final_buf));
            }
        }
        Err(_e) => { //todo: when is this branch taken?
            //just drop the client
            return Ok(DISCONNECT);
            //previous logic
            // return Err(e);
        }
    }
}
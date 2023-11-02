use std::net::SocketAddr;
use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use async_dup::Arc;
use smol::Async;

pub enum Event {
    Join(SocketAddr, Arc<Async<TcpStream>>),
    Leave(SocketAddr),
    Message(SocketAddr, Packet)
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum MessageType {
    STRING,
    IMAGE
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Packet {
    pub message_type: MessageType,
    pub data: Vec<u8>
}
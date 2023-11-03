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
#[derive(Clone)]
pub enum MessageType {
    STRING,
    IMAGE
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
#[derive(Clone)]
pub enum PktSource {
    CLIENT(SocketAddr), //set by server when messages are dispatched
    SERVER, //server messages, such as join/leave
    UNDEFINED //client should set source as undefined
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct Packet {
    pub src: PktSource,
    pub message_type: MessageType,
    pub data: Vec<u8>
}
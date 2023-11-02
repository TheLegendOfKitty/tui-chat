use smol::io::{AsyncWriteExt, BufReader};
use std::net::TcpStream;

use std::error::Error;
use postcard::to_allocvec;
use smol::{Async, io};
use crate::common::{MessageType, Packet};

pub mod common;
fn main() -> Result<(), Box<dyn Error>> {
    smol::block_on(async {
        // Open a TCP stream to the socket address
        let mut stream = Async::<TcpStream>::connect(([127, 0, 0, 1], 6000)).await?;
        println!("created stream");


        loop {
            let mut line= String::new();
            let _input = std::io::stdin().read_line(&mut line).unwrap();
            let _ = io::copy(
                BufReader::new(Vec::from(line.clone()).as_slice()),
                &stream).await;
            let result = stream.write(to_allocvec(&Packet {
                message_type: MessageType::STRING,
                data: Vec::from(line.clone()),
            }).unwrap().as_slice()).await;
            println!("wrote to stream; success={:?}", result.is_ok());
            line.clear();
        }
    })
}
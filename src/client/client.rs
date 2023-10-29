//! A TCP client.
//!
//! First start a server:
//!
//! ```
//! cargo run --example tcp-server
//! ```
//!
//! Then start a client:
//!
//! ```
//! cargo run --example tcp-client
//! ```
#[forbid(unsafe_code)]


use std::net::TcpStream;

use smol::{future, io, Async, Unblock};
use smol::io::BufReader;

async fn send(data: &[u8], stream: &Async<TcpStream>) -> io::Result<u64> {
    let data_reader = BufReader::new(data);
    io::copy(data_reader, stream).await
}

fn main() -> io::Result<()> {
    smol::block_on(async {
        // Create async stdin and stdout handles.
        let stdin = Unblock::new(std::io::stdin());
        let mut stdout = Unblock::new(std::io::stdout());

        // Connect to the server.
        let stream = Async::<TcpStream>::connect(([127, 0, 0, 1], 7000)).await?;
        println!("Connected to {}", stream.get_ref().peer_addr()?);
        println!("Type a message and hit enter!\n");

        let reader = &stream;
        let mut writer = &stream;

        let send_arr: &[u8] = b"Test";

        send(send_arr, &stream).await.unwrap();
        // Pipe messages from stdin to the server and pipe messages from the server to stdout.
        future::race(
            async {
                let result = io::copy(stdin, &mut writer).await;
                println!("Quit!");
                result
            },
            async {
                let result = io::copy(reader, &mut stdout).await;
                println!("Server Disconnected!");
                result
            },
        ).await?;

        Ok(())
    })
}
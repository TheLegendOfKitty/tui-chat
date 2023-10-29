//! A TCP server.
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

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream};

use async_channel::{bounded, Receiver, Sender};
use async_dup::Arc;
use smol::{io, prelude::*, Async};
/// An event on the chat server.
enum Event {
    /// A client has joined.
    Join(SocketAddr, Arc<Async<TcpStream>>),

    /// A client has left.
    Leave(SocketAddr),

    /// A client sent a message.
    Message(SocketAddr, String),
}

///Dispatch events to clients
async fn dispatch(receiver: Receiver<Event>) -> io::Result<()> {
    // Currently active clients.
    let mut client_map = HashMap::<SocketAddr, Arc<Async<TcpStream>>>::new();

    //Receive incoming events
    while let Ok(event) = receiver.recv().await {
        //Process the event and format a message to send to clients
        let output = match event {
            Event::Join(addr, stream) => {
                client_map.insert(addr, stream);
                format!("{} has joined\n", addr)
            }
            Event::Leave(addr) => {
                client_map.remove(&addr);
                format!("{} has left\n", addr)
            }
            Event::Message(addr, msg) => format!("{} says: {}t", addr, msg)
        };

        //Display the event in the server process
        print!("{}", output);

        //Send the event to all active clients
        for stream in client_map.values_mut() {
            // Ignore errors because the client may disconnect at any point
            stream.write_all(output.as_bytes()).await.ok();
        }
    }
    Ok(())
}


async fn read_messages(sender: Sender<Event>, client: Arc<Async<TcpStream>>) -> io::Result<()> {
    let addr = client.get_ref().peer_addr()?;
    let mut buf = [0; 1024]; //1 kilobytes
    let mut reader = io::BufReader::new(client);

    loop {
        let res = reader.read(&mut buf).await;
        match res {
            Ok(bytes_read) => {
                if bytes_read == 0 { //nothing read
                    break
                }
                let msg = String::from_utf8_lossy(&buf[..bytes_read]);
                sender.send(Event::Message(addr, msg.parse().unwrap())).await.ok();
            }
            Err(_) => {
                sender.send(Event::Leave(addr)).await.ok();
                break;
            }
        }

    }

    Ok(())
}

fn main() -> io::Result<()> {
    smol::block_on(async {
        // Create a listener.
        let listener = Async::<TcpListener>::bind(([127, 0, 0, 1], 7000))?;
        println!("Listening on {}", listener.get_ref().local_addr()?);
        println!("Now start a TCP client.");

        //Dispatcher channel
        let (dispatcher_channel_sender, dispatcher_channel_receiver) = bounded(100);
        //Spawn dispatcher
        smol::spawn(dispatch(dispatcher_channel_receiver)).detach();

        // Accept clients in a loop.
        loop {
            let (stream, peer_addr) = listener.accept().await?;
            let client = Arc::new(stream); //pointer to the new client stream
            let sender = dispatcher_channel_sender.clone(); //Clone the sender so as to not steal ownership of it when moving

            // Spawn a task that reads messages from the client
            smol::spawn(async move {
                //Client starts with a join event
                sender.send(Event::Join(peer_addr, client.clone())).await.ok();

                //Read messages from the client and ignore I/O errors when the client quits
                read_messages(sender.clone(), client).await.ok();

                //Client ends with a leave event
                sender.send(Event::Leave(peer_addr)).await.ok();

            }).detach();
        }
    })
}
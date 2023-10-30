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

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Terminal;
use tui_textarea::{Input, Key, TextArea};

use std::net::TcpStream;
use async_channel::bounded;
use futures_util::FutureExt;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};

use smol::{future, io, Async, Unblock};
use smol::io::{AsyncReadExt, BufReader};

//Unused as of now
async fn send(data: &[u8], stream: &Async<TcpStream>) -> io::Result<u64> {
    let data_reader = BufReader::new(data);
    io::copy(data_reader, stream).await
}

fn main() -> io::Result<()> {
    smol::block_on(async {
        // Create async stdin and stdout handles.
        let stdin_async = Unblock::new(std::io::stdin());
        let mut stdout_async = Unblock::new(std::io::stdout());

        // Connect to the server.
        let stream = Async::<TcpStream>::connect(([127, 0, 0, 1], 7000)).await?;
        println!("Connected to {}", stream.get_ref().peer_addr()?);
        println!("Type a message and hit enter!\n");

        //let reader = &stream;
        let mut writer = &stream;

        let (rec_buf_sender, rec_buf_receiver) = bounded(100);

        let mut messages_list = vec![];
        //start terminal prep

        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();

        enable_raw_mode()?;
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut term = Terminal::new(backend)?;

        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Crossterm Minimal Example"),
        );

        //end terminal prep

        // Pipe messages from stdin to the server and pipe messages from the server to stdout.
        future::race(
            async {
                let mut rec_buf : Vec<String> = vec![];
                loop {
                    term.draw(|f| {
                        let chunks = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                            .split(f.size());

                        f.render_widget(textarea.widget(), chunks[1]);

                        if let Some(msg) = rec_buf_receiver.recv().now_or_never(){
                            rec_buf.push(msg.unwrap());
                            for string in rec_buf.clone() {
                                messages_list.push(ListItem::new(string));
                            }
                        }

                        let items = List::new(messages_list.clone())
                            .block(Block::default().borders(Borders::ALL).title("List"))
                            .highlight_style(
                                Style::default()
                                    .bg(Color::LightGreen)
                                    .add_modifier(Modifier::BOLD),
                            )
                            .highlight_symbol(">> ");

                        f.render_widget(items, chunks[0]);
                    }).unwrap();

                    match crossterm::event::read().unwrap().into() {
                        Input {
                            key: Key::Esc, ..
                        } => {
                            let mut buf : Vec<u8>  = vec![];
                            for line in textarea.lines() {
                                buf.append(&mut line.clone().into_bytes().clone());
                            }
                            let data_reader = BufReader::new(buf.as_slice());
                            let result = io::copy(data_reader, &mut writer).await;
                            match result {
                                Ok(_) => {}
                                Err(_) => {break;}
                            }
                        }
                        input => {
                            textarea.input(input);
                        }
                    }
                }
                return 0;
            },
            async {
                let mut buf = [0; 1024]; //1 kilobytes
                let mut reader = BufReader::new(&stream);

                loop {
                    let res = reader.read(&mut buf).await;
                    match res {
                        Ok(bytes_read) => {
                            if bytes_read == 0 { //nothing read
                                break
                            }
                            let recv_msg = String::from_utf8_lossy(&buf[..bytes_read]);
                            rec_buf_sender.send(recv_msg.parse().unwrap()).await.unwrap();
                        }
                        Err(_) => {
                            println!("Server Disconnected!");
                            break;
                        }
                    }

                };
                return 0;
            },
        ).await;

        disable_raw_mode()?;
        crossterm::execute!(
        term.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
        )?;
        term.show_cursor()?;

        println!("Lines: {:?}", textarea.lines());

        Ok(())
    })
}
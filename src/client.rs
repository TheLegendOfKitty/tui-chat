#[forbid(unsafe_code)]

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;
use tui_textarea::{Input, Key, TextArea};

use smol::net::TcpStream;
//use std::net::TcpStream;

use std::error::Error;
use std::io;
use std::io::StdoutLock;
use std::net::{IpAddr, SocketAddr};
use futures_util::FutureExt;
use postcard::to_allocvec;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use smol::channel::{bounded};
use smol::io::{AsyncBufReadExt, BufReader, AsyncWriteExt};
use crate::common::{MessageType, Packet, PktSource};

pub mod common;

fn cleanup(mut term: Terminal<CrosstermBackend<StdoutLock>>) {
    disable_raw_mode().unwrap();
    crossterm::execute!(
            term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        ).unwrap();
    term.show_cursor().unwrap();
}
fn main() -> Result<(), Box<dyn Error>> {
    smol::block_on(async {
        // Open a TCP stream to the socket address
        let stream = TcpStream::connect(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 6000)).await.unwrap();

        let mut binding = stream.clone();
        let mut reader = BufReader::new(&mut binding);
        let mut writer = stream;

        let (message_channel_sender, message_channel_receiver) = bounded(100);

        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        enable_raw_mode().unwrap();
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture).unwrap();
        let backend = CrosstermBackend::new(stdout);
        let mut term = Terminal::new(backend).unwrap();

        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input"),
        );

        let mut lines = Vec::new();
        let mut consumed;
        'a: loop {
            match reader.fill_buf().now_or_never() {
                Some(Ok(bytes_read)) => {
                    //todo: will this ever happen?
                    if bytes_read.len() == 0 { //nothing read
                        continue 'a;
                    }
                    //todo: server can send bad data and crash client
                    let packet: Packet = postcard::from_bytes(bytes_read).unwrap();
                    //message_channel_sender.send(packet.clone()).await.ok();
                    consumed = bytes_read.len();
                    reader.consume(consumed);
                    message_channel_sender.send(packet).await.unwrap();
                    //let string = format!("{}", String::from_utf8(packet.data).unwrap());
                    //print!("{}", string);
                }
                Some(Err(e)) => {
                    //println!("Server Disconnected!");
                    cleanup(term);
                    panic!("{}", e);
                }
                _ => {}
            }

            term.draw(|f| {
                match message_channel_receiver.recv().now_or_never() {
                    None => {}
                    Some(res) => {
                        let packet = res.unwrap();
                        lines.push(Line::from(String::from_utf8(packet.data).unwrap()));
                    }
                }
                let main_layout = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(50), Constraint::Percentage(50)
                    ])
                    .split(f.size());
                f.render_widget(Paragraph::new(lines.clone()).block(Block::default().title("Messages").borders(Borders::ALL)), main_layout[0]);
                f.render_widget(textarea.widget(), main_layout[1]);
            }).unwrap();
            match crossterm::event::read().unwrap().into() {
                Input { key: Key::Esc, .. } => {
                    let packet = Packet {
                        src: PktSource::UNDEFINED,
                        message_type: MessageType::STRING,
                        data: Vec::from(textarea.lines().join("\n"))
                    };
                    writer.write(to_allocvec(&packet).unwrap().as_slice()).await.unwrap();
                },
                input => {
                    textarea.input(input);
                }
            }
        }




        //println!("Lines: {:?}", textarea.lines());
        /*future::race(
            async { //messages

            },
            async { //logic
                loop {
                    let mut line= String::new();
                    if let Some(msg) = message_channel_receiver.recv().now_or_never() {
                        println!("{}", String::from_utf8(msg.unwrap().data).unwrap())
                    }
                    match std::io::stdin().read_line(&mut line) {
                        Err(e) => {
                            return Err(e);
                        }
                        Ok(_read) => {
                            let result = writer.write(to_allocvec(&Packet {
                                src: PktSource::UNDEFINED,
                                message_type: MessageType::STRING,
                                data: Vec::from(line.clone()),
                            }).unwrap().as_slice()).await;
                            println!("wrote to stream; success={:?}", result.is_ok());
                            line.clear();
                        }
                    }
                }
            }
        ).await.unwrap();
*/

    });
    Ok(())
}

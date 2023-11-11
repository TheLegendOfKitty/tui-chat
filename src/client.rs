//! Client

#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::nursery, clippy::cargo)]
use crossterm::cursor;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use tui_textarea::{Input, Key, TextArea};

use std::net::TcpStream;

use std::error::Error;
use std::{fs, io};
use std::io::StdoutLock;
use std::net::{IpAddr, SocketAddr};
use futures_util::FutureExt;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::Line;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::{ResizeProtocol};
use ratatui_image::ResizeImage;
use smol::channel::{bounded, Receiver};
use smol::io::{BufReader};
use std::panic;
use core::time::Duration;
use std::ops::Deref;
use async_dup::Arc;
use crossterm::execute;
use smol::Async;

use crate::common::{MessageType, Packet, PktSource, read_data, ReadResult, send_with_header};

pub mod common;

struct App<'a> {
    terminal: Terminal<CrosstermBackend<StdoutLock<'a>>>,
    image: Option<Box<dyn ResizeProtocol>>,
    message_channel_receiver: Receiver<Packet>,
    lines: Vec<Line<'a>>,
    picker: Picker,
    textarea: TextArea<'a>
}

impl Drop for App<'_> {
    fn drop(&mut self) {
        cleanup(&mut self.terminal);
    }
}

fn destruct_terminal() {
    disable_raw_mode().unwrap();
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
    execute!(io::stdout(), cursor::Show).unwrap();
}

fn cleanup(term: &mut Terminal<CrosstermBackend<StdoutLock>>) {
    disable_raw_mode().unwrap();
    crossterm::execute!(
            term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        ).unwrap();
    term.show_cursor().unwrap();
}

fn main() -> Result<(), Box<dyn Error>> {
    panic::set_hook(Box::new(|panic_info| {
        destruct_terminal();
        better_panic::Settings::auto().create_panic_handler()(panic_info);
    }));


    smol::block_on(async {
        // Open a TCP stream to the socket address
        let stream = Async::<TcpStream>::connect(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 6000)).await.unwrap();

        let stream_ref = Arc::new(stream);
        let mut reader = BufReader::new(stream_ref.clone());
        let mut writer = stream_ref;

        let (message_channel_sender, message_channel_receiver) = bounded(100);

        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        //should be initialized before backend
        //let mut picker = Picker::from_termios(None).unwrap();
        let picker = Picker::new((10, 12), ProtocolType::Halfblocks, None).unwrap();

        enable_raw_mode().unwrap();
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture).unwrap();
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend).unwrap();


        let mut textarea = TextArea::default();
        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input"),
        );

        let lines = Vec::new();

        let mut app = App { terminal: term, image: None, message_channel_receiver, lines, picker, textarea };
        loop {
            //todo: is this leaking anything?
            match read_data(&mut reader).now_or_never() {
                None => {}
                Some(res) => {
                    match res.unwrap() {
                        ReadResult::EMPTY => {}
                        ReadResult::DISCONNECT => {
                            panic!("Server Disconnected!");
                        }
                        ReadResult::SUCCESS(bytes_read) => {
                            let packet: Packet = postcard::from_bytes(bytes_read.as_slice()).unwrap();
                            message_channel_sender.send(packet).await.unwrap();
                        }
                    }
                }
            }

            app.terminal.draw(|f| ui(f,
                                     &mut app.message_channel_receiver,
                                     &mut app.lines, &mut app.picker,
                                     &mut app.image, &mut app.textarea
            )).unwrap();
            let inpt = match crossterm::event::poll(Duration::from_secs(0)).unwrap() {
                true => {
                    crossterm::event::read().unwrap().into()
                }
                false => {
                    continue;
                }
            };
            match inpt {
                Input { key: Key::Char('x'),  .. } => 'a : {
                    let input_lines = app.textarea.lines();
                    if input_lines == [""] {
                        break 'a;
                    }
                    match input_lines.first().unwrap().deref()
                    .split(' ').collect::<Vec<&str>>().first().unwrap().deref() {
                        "/file" => {
                            let raw = fs::read(input_lines[0].split(' ').collect::<Vec<&str>>()[1]).unwrap();
                            let format = image::guess_format(raw.as_slice()).unwrap();
                            //guess_format does not verify validity of the entire memory - for now, we'll load the image into memory to verify its validity
                            let _dyn_img = image::load_from_memory_with_format(raw.as_slice(), format).unwrap();
                            let packet = Packet {
                                src: PktSource::UNDEFINED,
                                message_type: MessageType::IMAGE(format),
                                data: raw,
                            };
                            send_with_header(&mut writer, packet).await.unwrap();
                            break 'a;
                        }
                        "/exit" => {
                            return Ok(())
                        }
                        _ => {}
                    }
                    let packet = Packet {
                        src: PktSource::UNDEFINED,
                        message_type: MessageType::STRING,
                        data: Vec::from(input_lines.join("\n"))
                    };
                    send_with_header(&mut writer, packet).await.unwrap();
                },
                Input { key : Key::Char('c'), ctrl: true, .. } => {
                    return Ok(())
                },
                input => {
                    app.textarea.input(input);
                }
            }
        }
    })
}

fn ui<B: Backend>(f: &mut Frame<B>, app_message_channel_receiver: &mut Receiver<Packet>,
                  app_lines: &mut Vec<Line>, app_picker: &mut Picker,
                  app_image: &mut Option<Box<dyn ResizeProtocol>>, app_textarea: &mut TextArea
) {
    app_message_channel_receiver.recv().now_or_never().map_or(0, |res| {
        let packet = res.unwrap();
        match packet.message_type {
            MessageType::STRING => {
                app_lines.push(Line::from(String::from_utf8(packet.data).unwrap()));
            }
            MessageType::IMAGE(imgtype) => {
                let dyn_img = image::load_from_memory_with_format(packet.data.as_slice(), imgtype).unwrap();
                let image = app_picker.new_state(dyn_img);
                *app_image = Option::from(image);
            }
        }
        0
    });
    let main_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), Constraint::Percentage(50)
        ])
        .split(f.size());
    f.render_widget(Paragraph::new(app_lines.clone()).block(Block::default().title("Messages").borders(Borders::ALL)), main_layout[0]);
    f.render_widget(app_textarea.widget(), main_layout[1]);
    app_image.clone().map_or(0, |mut img| {
        f.render_stateful_widget(ResizeImage::new(None), main_layout[0], &mut img);
        0
    });
}
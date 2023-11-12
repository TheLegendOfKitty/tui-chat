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
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui_image::picker::{Picker};
use ratatui_image::protocol::{ResizeProtocol};
use ratatui_image::ResizeImage;
use smol::channel::{bounded, Receiver};
use smol::io::{BufReader};
use std::panic;
use core::time::Duration;
use std::fmt::{Debug, Formatter};
use std::ops::{Deref};
use std::rc::Rc;
use async_dup::Arc;
use crossterm::execute;
use qcell::{QCell, QCellOwner};
use smol::Async;

use crate::common::{MessageType, Packet, PktSource, read_data, ReadResult, send_with_header};

pub mod common;

const ESCAPE: Key = Key::Char('x');
const SEND: Key = Key::char('x');

#[derive(Clone)]
struct ImageWithData {
    img: Box<dyn ResizeProtocol>,
    _height: u32,
    _width: u32
}

#[derive(Clone)]
enum Menu<'a> {
    MessageInput(Rc<QCell<TextArea<'a>>>),
    ImageView
}

impl<'a> PartialEq for Menu<'a> {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn ne(&self, other: &Self) -> bool {
        !(std::mem::discriminant(self) == std::mem::discriminant(other))
    }
}

impl <'a> Debug for Menu<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Menu::MessageInput(_) => {
                write!(f, "Menu::MessageInput")
            }
            Menu::ImageView => {
                write!(f, "Menu::ImageView")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_menu_eq() {
        let owner1 = QCellOwner::new();
        let owner2 = QCellOwner::new();

        assert_eq!(Menu::MessageInput(Rc::new(QCell::new(&owner1, TextArea::default()))),
                   Menu::MessageInput(Rc::new(QCell::new(&owner2, TextArea::default())))
        )
    }
    #[test]
    fn test_menu_ne() {
        let owner1 = QCellOwner::new();

        assert_ne!(Menu::MessageInput(Rc::new(QCell::new(&owner1, TextArea::default()))),
                   Menu::ImageView
        )
    }
}

struct State<'a> {
    stack: Vec<Menu<'a>>,
    focus: Menu<'a>
}

impl<'a> State<'a> {
    fn push(&mut self, m: Menu<'a>) {
        self.stack.push(m);
    }

    fn pop(&mut self) -> Option<Menu<'a>> {
        self.stack.pop()
    }

    fn current(&self) -> Option<&Menu<'a>> {
        self.stack.last()
    }
}

struct App<'a> {
    terminal: Terminal<CrosstermBackend<StdoutLock<'a>>>,
    image: Option<ImageWithData>,
    message_channel_receiver: Receiver<Packet>,
    lines: Vec<Line<'a>>,
    picker: Picker,
    textarea: Rc<QCell<TextArea<'a>>>,
    textarea_owner: QCellOwner,
    zoom: u16,
    state: State<'a>
}

impl Drop for App<'_> {
    fn drop(&mut self) {
        graceful_cleanup(&mut self.terminal);
    }
}

fn unexpected_cleanup() {
    disable_raw_mode().unwrap();
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture).unwrap();
    execute!(io::stdout(), cursor::Show).unwrap();
}

fn graceful_cleanup(term: &mut Terminal<CrosstermBackend<StdoutLock>>) {
    disable_raw_mode().unwrap();
    crossterm::execute!(
            term.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        ).unwrap();
    term.show_cursor().unwrap();
}

fn exit(app: App) -> ! {
    drop(app);
    std::process::exit(0);
}

fn main() -> Result<(), Box<dyn Error>> {
    panic::set_hook(Box::new(|panic_info| {
        unexpected_cleanup();
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
        #[cfg(feature = "protocol_auto")]
        let picker = Picker::from_termios(None).unwrap();

        #[cfg(feature = "protocol_halfblocks")]
        let picker = Picker::new((10, 12), ProtocolType::Halfblocks, None).unwrap();

        #[cfg(not(any(feature = "protocol_auto", feature = "protocol_halfblocks")))]
        compile_error!("Neither protocol_auto nor protocol_halfblocks features were set!");

        enable_raw_mode().unwrap();
        crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture).unwrap();
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend).unwrap();

        let mut textarea_owner = QCellOwner::new();

        let textarea = Rc::new(QCell::new(&textarea_owner, TextArea::default()));

        textarea.rw(&mut textarea_owner).set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input"),
        );

        let lines = Vec::new();

        let mut app = App { terminal: term, image: None, message_channel_receiver, lines, picker, textarea: textarea.clone(), textarea_owner,
            zoom: 35, state: State {
            stack: vec![Menu::MessageInput(textarea.clone())],
            focus: Menu::MessageInput(textarea),
        }};
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

            message_recv(&app.message_channel_receiver, &mut app.lines,
                         &mut app.picker, &mut app.image, &mut app.state);

            app.terminal.draw(|f| ui(f, &mut app.lines,
                                     &mut app.image, &mut app.textarea,
                                     &app.textarea_owner, &app.zoom, &app.state
            )).unwrap();

            let inpt : Input = match crossterm::event::poll(Duration::from_secs(0)).unwrap() {
                true => {
                    crossterm::event::read().unwrap().into()
                }
                false => {
                    continue;
                }
            };
            app = handle_input(inpt, &mut writer, app).await;
        }
    })
}

fn ui<B: Backend>(f: &mut Frame<B>, app_lines: &mut Vec<Line>,
                  app_image: &mut Option<ImageWithData>, app_textarea: &mut Rc<QCell<TextArea>>,
                  app_textarea_owner: &QCellOwner,
                  app_zoom: &u16, app_state: &State)
{
    let main_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), Constraint::Percentage(50)
        ])
        .split(f.size());
    f.render_widget(Paragraph::new(app_lines.clone()).block(Block::default().title("Messages").borders(Borders::ALL)), main_layout[0]);
    f.render_widget(app_textarea.ro(app_textarea_owner).widget(), main_layout[1]);

    app_image.clone().map_or(0, |mut img| {
        if app_state.focus == Menu::ImageView {
            let img_layout = Block::default().borders(Borders::ALL).title("Image");
            f.render_stateful_widget(ResizeImage::new(None),
                                     img_layout.inner(centered_rect(f.size(), *app_zoom, *app_zoom)),
                                     &mut img.img);
            f.render_widget(img_layout, centered_rect(f.size(), *app_zoom, *app_zoom));
        }
        0
    });

}

fn message_recv(app_message_channel_receiver: &Receiver<Packet>, app_lines: &mut Vec<Line>,
                app_picker: &mut Picker, app_image: &mut Option<ImageWithData>,
                app_state: &mut State)
{
    app_message_channel_receiver.recv().now_or_never().map_or(0, |res| {
        let packet = res.unwrap();
        match packet.message_type {
            MessageType::STRING => {
                app_lines.push(Line::from(String::from_utf8(packet.data).unwrap()));
            }
            MessageType::IMAGE(imgtype) => {
                let dyn_img = image::load_from_memory_with_format(packet.data.as_slice(), imgtype).unwrap();
                let image = app_picker.new_state(dyn_img.clone());
                *app_image = Option::from(ImageWithData {
                    img: image,
                    _height: dyn_img.height(),
                    _width: dyn_img.width(),
                });
                app_state.focus = Menu::ImageView;
                app_state.push(Menu::ImageView);
            }
        }
        0
    });
}

fn centered_rect(r: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/*static handle_input: fn(Input, &mut Arc<Async<TcpStream>>, &mut App) = move*/
async fn handle_input<'a>(inpt: Input, writer: &mut Arc<Async<TcpStream>>, mut app: App<'a>) -> App<'a> {
    return match app.state.focus.clone() {
        Menu::MessageInput(textarea) => {
            match messages_input(inpt, writer, textarea, &mut app.textarea_owner).await {
                Ok(_) => {
                    app
                }
                Err(_) => {
                    exit(app);
                }
            }
        }
        Menu::ImageView => {
            images_input(inpt, &mut app.zoom, &mut app.state).await;
            app
        }
    }
}

async fn images_input(inpt: Input, app_zoom: &mut u16, app_state: &mut State<'_>) {
    match inpt {
        Input { key : Key::Char('+'), /*ctrl: true,*/ .. } => {
            if *app_zoom < 100 {
                *app_zoom += 5;
            }
        },
        Input { key : Key::Char('-'), /*ctrl: true,*/ .. } => {
            if *app_zoom > 35 {
                *app_zoom -= 5;
            }
        },
        Input { key : ESCAPE, .. } => {
            app_state.pop().unwrap();
            app_state.focus = app_state.current().unwrap().clone();
        }
        _input => {

        }
    }
}

async fn messages_input(inpt: Input, writer: &mut Arc<Async<TcpStream>>, text_area: Rc<QCell<TextArea<'_>>>, text_area_owner: &mut QCellOwner) -> Result<(), ()> {
    match inpt {
        Input { key: SEND,  .. } => 'a : {
            let input_lines = text_area.ro(text_area_owner).lines();
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
                    send_with_header(writer, packet).await.unwrap();
                    break 'a;
                }
                "/exit" => {
                    return Err(());
                }
                _ => {}
            }
            let packet = Packet {
                src: PktSource::UNDEFINED,
                message_type: MessageType::STRING,
                data: Vec::from(input_lines.join("\n"))
            };
            send_with_header(writer, packet).await.unwrap();
        },
        Input { key : Key::Char('c'), ctrl: true, .. } => {
            return Err(());
        },
        input => {
            text_area.rw(text_area_owner).input(input);
        }
    }
    return Ok(())
}

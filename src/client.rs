//! Client

#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::nursery, clippy::cargo)]
#![allow(clippy::needless_return)]
#![feature(let_chains)]

#[cfg(feature = "protocol_halfblocks")]
use ratatui_image::picker::ProtocolType;
use crossterm::cursor;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::{Frame, Terminal};
use tui_textarea::{Input, Key, TextArea};

use std::net::TcpStream;

use std::error::Error;
use std::{fs, io};
use std::io::StdoutLock;
use std::net::{IpAddr, SocketAddr};
use futures_util::FutureExt;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui_image::picker::{Picker};
use ratatui_image::protocol::{ResizeProtocol};
use ratatui_image::ResizeImage;
use smol::channel::{bounded, Receiver};
use smol::io::{BufReader};
use std::panic;
use core::time::Duration;
use std::fmt::{Debug, Formatter};
use std::ops::{Deref};
use std::path::Path;
use async_dup::Arc;
use crossterm::execute;
use ratatui::style::{Color, Modifier, Style};
use smol::Async;

use crate::common::{MessageType, Packet, PktSource, read_data, ReadResult, send_with_header, ImageData};

pub mod common;

static ESCAPE: Input = Input { key: Key::Char('x'), ctrl: false, alt: false };
static SEND: Input = Input { key: Key::Char('x'), ctrl: false, alt: false };
static IMAGE_ZOOM_IN: Input = Input { key : Key::Char('+'), /*ctrl: true,*/ ctrl: false, alt: false };
static IMAGE_ZOOM_OUT: Input = Input { key : Key::Char('-'), /*ctrl: true,*/ ctrl: false, alt: false };
static _IMAGE_MOVE_LEFT: Input = Input { key: Key::Left, ctrl: false, alt: false };
static _IMAGE_MOVE_RIGHT: Input = Input { key: Key::Right, ctrl: false, alt:false };
static _IMAGE_MOVE_UP: Input = Input { key: Key::Up, ctrl: false, alt: false };
static _IMAGE_MOVE_DOWN: Input = Input { key: Key::Down, ctrl: false, alt: false };
static SHIFT_TO_MESSAGES: Input = Input { key: Key::Char('<'), ctrl: false, alt: false };
static SHIFT_TO_INPUT: Input = Input { key: Key::Char('>'), ctrl: false, alt: false };
static MESSAGE_SELECT: Input = Input { key: Key::Enter, ctrl: false, alt: false };
const POLL_TIME: Duration = Duration::from_millis(50);

struct StatefulList<T> {
    state: ListState,
    items: Vec<T>,
}

impl<T> StatefulList<T> {
    fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn current(&self) -> &T {
        &self.items[self.state.selected().unwrap()]
    }

    fn unselect(&mut self) {
        self.state.select(None);
    }
}

#[derive(Clone)]
struct ImageWithData {
    img: Box<dyn ResizeProtocol>,
    _name: String,
    _height: u32,
    _width: u32
}

enum Menu<'a> {
    MessageInput(TextArea<'a>),
    ImageView,
    MessageList(StatefulList<Packet>)
}

impl<'a> PartialEq for Menu<'a> {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/*impl<'a> PartialEq<Menu> for Menu<'a> {
    fn eq(&self, other: &Menu) -> bool {
        match other {
            Menu::MessageInput(_) => {
                *self == Menu::MessageInput
            }
            Menu::ImageView => {}
            Menu::MessageList(_) => {}
        }
    }
}*/

impl <'a> Debug for Menu<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Menu::MessageInput(_) => {
                write!(f, "Menu::MessageInput")
            }
            Menu::ImageView => {
                write!(f, "Menu::ImageView")
            }
            Menu::MessageList(_) => {
                write!(f, "Menu::MessageList")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_menu_eq() {
        assert_eq!(Menu::MessageInput(TextArea::default()),
                   Menu::MessageInput(TextArea::default())
        )
    }
    #[test]
    fn test_menu_ne() {
        assert_ne!(Menu::MessageInput(TextArea::default()),
                   Menu::ImageView
        )
    }
}

/*
            MessageInput
                 |
MessageList <----------> ImageView
*/
struct State<'a> {
    stack: Vec<Menu<'a>>,
    _cursor_visible: bool
}

impl<'a> State<'a> {
    fn push(&mut self, m: Menu<'a>) {
        self.stack.push(m);
    }

    fn pop(&mut self) -> Option<Menu<'a>> {
        self.stack.pop()
    }

    fn focus(&self) -> Option<&Menu<'a>> {
        self.stack.last()
    }

    fn _focus_mut(&mut self) -> Option<&mut Menu<'a>> {
        self.stack.last_mut()
    }

    fn shift_back(&mut self) -> Menu<'a> {
        self.pop().unwrap()
    }

    fn shift_state(&mut self, menu: Menu<'a>) {
        self.push(menu);
    }

    fn base(&self) -> &TextArea {
        match self.stack.first().unwrap() {
            Menu::MessageInput(area) => {
                area
            }
            _ => {
                panic!("Base of stack was not a TextArea!")
            }
        }
    }

}

struct App<'a> {
    terminal: Terminal<CrosstermBackend<StdoutLock<'a>>>,
    image: Option<ImageWithData>,
    message_channel_receiver: Receiver<Packet>,
    picker: Picker,
    zoom_x: u16,
    zoom_y: u16,
    state: State<'a>,
    messages: Option<StatefulList<Packet>>,
}

impl<'a> App<'a> {
    /*fn textarea(&self) -> &TextArea {
        return self.state.base()
    }
    fn textarea_mut(&'a mut self) -> &mut TextArea {
        return self.state.base_mut();
    }*/

    fn messages_mut(&mut self) -> &mut StatefulList<Packet> {
        match &mut self.messages {
            None => {
                for menu in self.state.stack.iter_mut() {
                    if let Menu::MessageList(list) = menu {
                        return list
                    }
                }
            }
            Some(list) => {
                return list
            }
        }
        panic!("No message list found!")
    }

    fn messages(&self) -> &StatefulList<Packet> {
        match &self.messages {
            None => {
                for menu in self.state.stack.iter() {
                    if let Menu::MessageList(list) = menu {
                        return list
                    }
                }
            }
            Some(list) => {
                return list
            }
        }
        panic!("No message list found!")
    }
    /*fn base_mut(&mut self) -> &mut TextArea {
        match self.state.stack.first_mut().unwrap() {
            Menu::MessageInput(area) => {
                area
            }
            _ => {
                panic!("Base of stack was not a TextArea!")
            }
        }
    }*/
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

fn _exit(app: App) -> ! {
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

        //let mut textarea_owner = QCellOwner::new();

        let mut textarea = TextArea::default();

        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input")
                .style(Style::default()
                    .fg(Color::Blue))
        );

        //let messages_owner = QCellOwner::new();

        let mut app = App { terminal: term, image: None, message_channel_receiver, picker,
            zoom_x: 35, zoom_y: 35, state: State {
            stack: vec![Menu::MessageInput(textarea)],
                _cursor_visible: true
        },
            messages: Some(StatefulList::with_items(Vec::new()))
        };
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

            message_recv(&app.message_channel_receiver, &mut app.messages,
                         &mut app.picker, &mut app.image, &mut app.state);

            //let app_textarea = if let Menu::MessageInput(x) = &mut app.state.stack[0] { x } else { panic!() };

            app.terminal.draw(|f| ui(f, &mut app.messages,
                                     &app.zoom_x, &app.zoom_y, &mut app.image,&mut app.state
            )).unwrap();


            let inpt : Input = match crossterm::event::poll(POLL_TIME).unwrap() {
                true => {
                    crossterm::event::read().unwrap().into()
                }
                false => {
                    continue;
                }
            };
            handle_input(inpt, &mut writer, &mut app).await;
        }
    })
}

#[allow(clippy::ptr_arg)]
fn ui<B: Backend>(f: &mut Frame<B>, app_messages: &mut Option<StatefulList<Packet>>,
                  app_zoom_x: &u16, app_zoom_y: &u16, app_image: &mut Option<ImageWithData>, app_state: &mut State)
{
    let messages;
    match app_messages {
        None => 'a : {
            for menu in app_state.stack.iter_mut() {
                if let Menu::MessageList(list) = menu {
                    messages = list;
                    break 'a;
                }
            }
            panic!("No message list found!")
        }
        Some(list) => {
            messages = list
        }
    }

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(80), Constraint::Percentage(20)
        ])
        .split(f.size());

    let messages_vec: Vec<ListItem> = messages.items.iter()
        .map(|packet| {
            match &packet.message_type {
                MessageType::STRING => {
                    ListItem::new(String::from_utf8(packet.data.clone()).unwrap())
                }
                MessageType::IMAGE(img) => {
                    ListItem::new(format!("Image of type {:?}, {}", img.format, img.name))
                }
            }.style(Style::default().fg(Color::White).bg(Color::Black))
        }).collect();

    let messages_list = List::new(messages_vec)
        .block(Block::default().title("Messages").borders(Borders::ALL))
        .highlight_style(
            Style::default()
                .bg(Color::LightGreen)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(messages_list,main_layout[0], &mut messages.state);

    let app_textarea = if let Menu::MessageInput(x) = &mut app_state.stack[0] { x } else { panic!() };
    f.render_widget(app_textarea.widget(), main_layout[1]);

    let _u = app_image.as_mut().map_or(0, |img| {
        if *app_state.focus().unwrap() == Menu::ImageView {
            let img_layout = Block::default().borders(Borders::ALL).title("Image");
            f.render_stateful_widget(ResizeImage::new(None),
                                     img_layout.inner(centered_rect(f.size(), *app_zoom_x, *app_zoom_y)),
                                     &mut img.img);
            f.render_widget(img_layout, centered_rect(f.size(), *app_zoom_x, *app_zoom_y));
        }
        0
    });

}

fn message_recv(app_message_channel_receiver: &Receiver<Packet>, app_messages: &mut Option<StatefulList<Packet>>,
                app_picker: &mut Picker, app_image: &mut Option<ImageWithData>,
                app_state: &mut State)
{
    let messages ;
    match app_messages {
        None => 'a : {
            for menu in app_state.stack.iter_mut() {
                if let Menu::MessageList(list) = menu {
                    messages = list;
                    break 'a;
                }
            }
            panic!("No message list found!")
        }
        Some(list) => {
            messages = list
        }
    }

    match app_message_channel_receiver.recv().now_or_never() {
        None => {}
        Some(res) => {
            let packet = res.unwrap();
            messages.items.push(packet.clone());
            match messages.items.last().unwrap().message_type {
                MessageType::STRING => {
                    //app_lines.push(Line::from(String::from_utf8(packet.data).unwrap()));
                }
                MessageType::IMAGE(ref imgtype) => {
                    let dyn_img = image::load_from_memory_with_format(messages.items.last().unwrap().data.as_slice(), imgtype.format).unwrap();
                    let image = app_picker.new_state(dyn_img.clone());
                    *app_image = Option::from(ImageWithData {
                        img: image,
                        _name: imgtype.name.clone(),
                        _height: dyn_img.height(),
                        _width: dyn_img.width(),
                    });
                    app_state.shift_state(Menu::ImageView);
                }
            }
        }
    };

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
async fn handle_input<'a>(inpt: Input, writer: &mut Arc<Async<TcpStream>>, app: &mut App<'a>) {
    match app.state.focus().unwrap() {
        Menu::MessageInput(_) => {
            match messages_input(inpt, writer, app).await
            {
                Ok(_) => {
                    //app
                }
                Err(_) => {
                    //exit(app)
                }
            }
        }
        Menu::ImageView => {
            images_input(inpt, &mut app.zoom_x, &mut app.zoom_y, &mut app.state).await;
            //app
        }
        Menu::MessageList(_) => {
            messages_list_input(inpt, app).await;
            //app
        }
    }
}

//todo: does this need to be async?
async fn messages_list_input(inpt: Input, app: & mut App<'_>)
{
    match inpt {
        _ if inpt == SHIFT_TO_INPUT => {
            app.messages_mut().unselect();
            if let Menu::MessageInput(area) = &mut app.state.stack[0] {
                area.set_block(Block::default()
                    .borders(Borders::ALL)
                    .title("Input")
                    .style(Style::default()
                        .fg(Color::Blue)));
            };
            let _ = std::mem::replace(&mut app.messages, Some(
                if let Menu::MessageList(list) = app.state.shift_back() { list } else { panic!() }
            ));
        }
        Input { key: Key::Up, .. } => {
            app.messages_mut().previous();
        }
        Input { key: Key::Down, .. } => {
            app.messages_mut().next();
        }
        _ if inpt == MESSAGE_SELECT => {
            let packet = app.messages().current();
            match &packet.message_type {
                MessageType::STRING => {

                }
                MessageType::IMAGE(imagedata) => {
                    let name = imagedata.name.clone();
                    let dyn_img = image::load_from_memory_with_format(packet.data.as_slice(), imagedata.format).unwrap();
                    let image = app.picker.new_state(dyn_img.clone());
                    app.image = Option::from(ImageWithData {
                        img: image,
                        _name: name,
                        _height: dyn_img.height(),
                        _width: dyn_img.width(),
                    });
                    app.state.shift_state(Menu::ImageView);
                }
            }
        }
        Input { .. } => {}
    }
}

//todo: does this need to be async?
async fn images_input(inpt: Input, app_zoom_x: &mut u16, app_zoom_y: &mut u16, app_state: &mut State<'_>) {
    match inpt {
         _ if inpt == IMAGE_ZOOM_IN => {
            if *app_zoom_x < 100 && *app_zoom_y < 100 {
                *app_zoom_x += 5;
                *app_zoom_y += 5;
            }
        }
        _ if inpt == IMAGE_ZOOM_OUT => {
            if *app_zoom_x > 35 && *app_zoom_y > 35 {
                *app_zoom_x -= 5;
                *app_zoom_y -= 5;
            }
        },
        _ if inpt == ESCAPE => {
            /*app_state.pop().unwrap();
            app_state.focus = app_state.current().unwrap().clone();*/
            app_state.shift_back();
        }
        _input => {

        }
    }
}

async fn messages_input(inpt: Input, writer: &mut Arc<Async<TcpStream>>, app: &mut App<'_>) -> Result<(), ()>
{
    match inpt {
        _ if inpt == SEND => 'a : {
            let input_lines = app.state.base().lines();
            if input_lines == [""] {
                break 'a;
            }
            #[allow(suspicious_double_ref_op)]
            match input_lines.first().unwrap().deref()
                .split(' ').collect::<Vec<&str>>().first().unwrap().deref() {
                "/file" => {
                    let path = Path::new(input_lines[0].split(' ').collect::<Vec<&str>>()[1]);
                    let name : String = path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap();
                    let raw = fs::read(path).unwrap();
                    let format = image::guess_format(raw.as_slice()).unwrap();
                    //guess_format does not verify validity of the entire memory - for now, we'll load the image into memory to verify its validity
                    let _dyn_img = image::load_from_memory_with_format(raw.as_slice(), format).unwrap();
                    let packet = Packet {
                        src: PktSource::UNDEFINED,
                        message_type: MessageType::IMAGE(ImageData {
                            format, name}),
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
        _ if inpt == SHIFT_TO_MESSAGES => {
            app.messages_mut().state.select(Some(0));
            {
                if let Menu::MessageInput(area) = &mut app.state.stack[0] {
                    area.set_block(Block::default()
                        .borders(Borders::ALL)
                        .title("Input")
                        .style(Style::default()));
                }
            }
            let message_list = std::mem::replace(&mut app.messages, None);
            app.state.shift_state(Menu::MessageList(message_list.unwrap()));
        }
        Input { key : Key::Char('c'), ctrl: true, .. } => {
            return Err(());
        },
        input => {
            if let Menu::MessageInput(area) = &mut app.state.stack[0] {
                area.input(input);
            }
        }
    }
    return Ok(())
}


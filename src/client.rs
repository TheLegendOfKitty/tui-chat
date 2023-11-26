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
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Widget, Wrap};
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
use std::path::{Path, PathBuf};
use async_dup::Arc;
use crossterm::execute;
use file_format::FileFormat;
use ratatui::style::{Color, Modifier, Style};
use smol::Async;

use crate::common::{MessageType, Packet, PktSource, read_data, ReadResult, send_with_header, ClientList, Client, ImageData};

pub mod common;

const SEND: Input = Input { key: Key::Char('x'), ctrl: false, alt: false };
const IMAGE_ZOOM_IN: Input = Input { key : Key::Char('+'), /*ctrl: true,*/ ctrl: false, alt: false };
const IMAGE_ZOOM_OUT: Input = Input { key : Key::Char('-'), /*ctrl: true,*/ ctrl: false, alt: false };
const _IMAGE_MOVE_LEFT: Input = Input { key: Key::Left, ctrl: false, alt: false };
const _IMAGE_MOVE_RIGHT: Input = Input { key: Key::Right, ctrl: false, alt:false };
const _IMAGE_MOVE_UP: Input = Input { key: Key::Up, ctrl: false, alt: false };
const _IMAGE_MOVE_DOWN: Input = Input { key: Key::Down, ctrl: false, alt: false };
const SHIFT_BACK: Input = Input { key: Key::Char('<'), ctrl: false, alt: false };
const MESSAGE_SELECT: Input = Input { key: Key::Enter, ctrl: false, alt: false };
const SHIFT_TO_PEERS: Input = Input { key: Key::Char('P'), ctrl: false, alt: false};
const SHIFT_TO_MESSAGES_LIST : Input = Input { key: Key::Char('L'), ctrl: false, alt: false };
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

struct FileBrowser {
    current_dir_list: StatefulList<PathBuf>,
    current_dir: PathBuf
}

struct Popup {
    title: String,
    content: String
}
enum Menu<'a> {
    MessageInput(TextArea<'a>),
    ImageView,
    MessageList(StatefulList<Packet>),
    PeerList(StatefulList<Client>),
    FileBrowser(FileBrowser),
    Popup(Popup)
}

impl<'a> PartialEq for Menu<'a> {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
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
            Menu::MessageList(_) => {
                write!(f, "Menu::MessageList")
            }
            Menu::PeerList(_) => {
                write!(f, "Menu::PeerList")
            }
            Menu::FileBrowser(_) => {
                write!(f, "Menu::FileBrowser")
            }
            Menu::Popup(_) => {
                write!(f, "Menu::Popup")
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
    stack: Vec<(Menu<'a>, Box<dyn FnOnce(&mut Menu, &mut State)>)>,
    _cursor_visible: bool
}

impl<'a> State<'a> {
    fn push(&mut self, m: Menu<'a>, handler: Box<dyn FnOnce(&mut Menu, &mut State)>) {
        self.stack.push((m, handler));
    }

    fn pop(&mut self) -> Option<(Menu<'a>, Box<dyn FnOnce(&mut Menu, &mut State)>)> {
        self.stack.pop()
    }

    fn focus(&self) -> &Menu<'a> {
        &self.stack.last().unwrap().0
    }

    fn _focus_mut(&mut self) -> &mut Menu<'a> {
        &mut self.stack.last_mut().unwrap().0
    }

    fn shift_state(&mut self, menu: Menu<'a>, return_logic: impl FnOnce(&mut Menu, &mut State) + 'static) {
        self.push(menu, Box::new(return_logic));
    }

    fn shift_back(&mut self) -> Menu<'a> {
        let mut returned = self.pop().unwrap();
        returned.1(&mut returned.0, self);
        returned.0
    }

    fn base(&self) -> &TextArea {
        match &self.stack.first().unwrap().0 {
            Menu::MessageInput(area) => {
                area
            }
            _ => {
                panic!("Base of stack was not a TextArea!")
            }
        }
    }

    fn file_browser_mut(&mut self) -> Option<&mut FileBrowser> {
        for menu in self.stack.iter_mut() {
            if let Menu::FileBrowser(filebrowser) = &mut menu.0 {
                return Some(filebrowser)
            }
        }
        return None;
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
    peers: Option<StatefulList<Client>>
}

impl<'a> App<'a> {
    fn messages_mut(&mut self) -> &mut StatefulList<Packet> {
        match &mut self.messages {
            None => {
                for menu in self.state.stack.iter_mut() {
                    if let Menu::MessageList(list) = &mut menu.0 {
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
                    if let Menu::MessageList(list) = &menu.0 {
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

    fn peers_mut(&mut self) -> &mut StatefulList<Client> {
        match &mut self.peers {
            None => {
                for menu in self.state.stack.iter_mut() {
                    if let Menu::PeerList(list) = &mut menu.0 {
                        return list;
                    }
                }
                panic!("No peer list found!")
            }
            Some(list) => {
                return list;
            }
        }
    }
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

        let mut textarea = TextArea::default();

        textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .title("Input")
                .style(Style::default()
                    .fg(Color::Blue))
        );

        let mut app = App { terminal: term, image: None, message_channel_receiver, picker,
            zoom_x: 35, zoom_y: 35, state: State {
            stack: vec![(Menu::MessageInput(textarea), Box::new(|_, _| {panic!("Shifted state away from base text area!")}))],
                _cursor_visible: true
        },
            messages: Some(StatefulList::with_items(Vec::new())),
            peers: Some(StatefulList::with_items(Vec::new()))
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

            message_recv(&app.message_channel_receiver, &mut app.messages, &mut app.picker, &mut app.image, &mut app.state, &mut app.peers);

            app.terminal.draw(|f| ui(f, &mut app.messages,
                                     &app.zoom_x, &app.zoom_y, &mut app.image,&mut app.state, &mut app.peers
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
                  app_zoom_x: &u16, app_zoom_y: &u16, app_image: &mut Option<ImageWithData>, app_state: &mut State, app_peers: &mut Option<StatefulList<Client>>)
{

    match app_state.file_browser_mut() {
        Some(browser) => {
            let layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(70), Constraint::Percentage(30)
                ])
                .split(f.size());


            let current_list = List::new(
                browser.current_dir_list.items.iter().map(|entry| {
                        ListItem::new(entry.clone().into_os_string().into_string().unwrap()).style(Style::default().fg(Color::White).bg(Color::Black))
                      }).collect::<Vec<ListItem>>()
            )
                .block(Block::default().title(browser.current_dir.to_str().unwrap()).borders(Borders::ALL))
                .style(Style::default())
                .highlight_style(
                    Style::default()
                        .bg(Color::LightGreen)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(">> ");

            f.render_stateful_widget(current_list, layout[0], &mut browser.current_dir_list.state);

            let right_section = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(10), Constraint::Percentage(30), Constraint::Percentage(60)
                ])
                .split(layout[1]);

            let top_right_section = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(25), Constraint::Percentage(50), Constraint::Percentage(25)
                ])
                .split(right_section[1]);

            let b = Block::default()
                .title("          ")
                .borders(Borders::ALL);

            f.render_widget(b, top_right_section[1]);

            let current_file = browser.current_dir_list.current();

            let mut paragraph = Paragraph::default();
            if current_file.is_dir() {
                paragraph = Paragraph::new("Directory");
            }
            else if current_file.is_symlink() {
                paragraph = Paragraph::new(format!("Symlink to {}", fs::read_link(current_file).unwrap().into_os_string().into_string().unwrap()));
            }
            else if current_file.is_file() {
                let format = FileFormat::from_file(current_file).unwrap();

                paragraph = Paragraph::new(format.to_string());
                //let metadata = fs::metadata(current_file).unwrap();
                //metadata.file_type()
            }
            else {
                //panic!("{}", current_file.clone().into_os_string().into_string().unwrap());
            }
            f.render_widget(paragraph,
                            right_section[0]);
        }
        None => {
            let main_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(80), Constraint::Percentage(20)
                ])
                .split(f.size());

            let top_section = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(80), Constraint::Percentage(20)
                ])
                .split(main_layout[0]);

            let messages;
            match app_messages {
                None => 'a : {
                    for menu in app_state.stack.iter_mut() {
                        if let Menu::MessageList(list) = &mut menu.0 {
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

            let mut messages_vec : Vec<ListItem> = Vec::new();
            for packet in messages.items.iter() {
                match &packet.message_type {
                    MessageType::STRING => {
                        messages_vec.push(
                            ListItem::new(String::from_utf8(packet.data.clone()).unwrap()).style(Style::default().fg(Color::White).bg(Color::Black)));
                    }
                    MessageType::IMAGE(img) => {
                        messages_vec.push(
                            ListItem::new(format!("Image of type {:?}, {}", img.format, img.name)).style(Style::default().fg(Color::White).bg(Color::Black)));
                    }
                    MessageType::CLIENTS => {}
                }
            }

            let messages_list = List::new(messages_vec)
                .block(Block::default().title("Messages").borders(Borders::ALL))
                .highlight_style(
                    Style::default()
                        .bg(Color::LightGreen)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(">> ");

            f.render_stateful_widget(messages_list,top_section[0], &mut messages.state);

            let peers;
            match app_peers {
                None => 'a : {
                    for menu in app_state.stack.iter_mut() {
                        if let Menu::PeerList(list) = &mut menu.0 {
                            peers = list;
                            break 'a;
                        }
                    }
                    panic!("No peer list found!")
                }
                Some(list) => {
                    peers = list;
                }
            }

            let mut peers_vec : Vec<ListItem> = Vec::new();
            for peer in peers.items.iter() {
                peers_vec.push(
                    ListItem::new(peer.addr.to_string()).style(Style::default().fg(Color::White).bg(Color::Black))
                );
            }

            let peers_list = List::new(peers_vec)
                .block(Block::default().title("Peers").borders(Borders::ALL))
                .highlight_style(
                    Style::default()
                        .bg(Color::LightGreen)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol(">> ");

            f.render_stateful_widget(peers_list, top_section[1], &mut peers.state);

            /*messages_vec.push(
                ListItem::new(
                    format!("{:?}",
                            app_peers.clients.iter().map(|client| {
                                client.addr.to_string()
                            }).collect::<Vec<String>>()
                    )
                ).style(Style::default().fg(Color::White).bg(Color::Black)));*/

            let app_textarea = if let Menu::MessageInput(x) = &mut app_state.stack[0].0 { x } else { panic!() };
            f.render_widget(app_textarea.widget(), main_layout[1]);

            let _u = app_image.as_mut().map_or(0, |img| {
                if *app_state.focus() == Menu::ImageView {
                    let img_layout = Block::default().borders(Borders::ALL).title("Image");
                    f.render_stateful_widget(ResizeImage::new(None),
                                             img_layout.inner(centered_rect(f.size(), *app_zoom_x, *app_zoom_y)),
                                             &mut img.img);
                    f.render_widget(img_layout, centered_rect(f.size(), *app_zoom_x, *app_zoom_y));
                }
                0
            });
        }
    };
    if let Menu::Popup(p) = app_state.focus() {
        f.render_widget(Paragraph::new(p.content.deref())
                            .wrap(Wrap {trim: false})
                            .block(Block::default().title(p.title.deref()).borders(Borders::ALL)), centered_rect(f.size(), 35, 35));
    }

}

fn message_recv(app_message_channel_receiver: &Receiver<Packet>, app_messages: &mut Option<StatefulList<Packet>>,
                app_picker: &mut Picker, app_image: &mut Option<ImageWithData>,
                app_state: &mut State, app_peers: &mut Option<StatefulList<Client>>)
{
    let messages ;
    match app_messages {
        None => 'a : {
            for menu in app_state.stack.iter_mut() {
                if let Menu::MessageList(list) = &mut menu.0 {
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
                    app_state.shift_state(Menu::ImageView, |_, _| {/* nothing */});
                }
                MessageType::CLIENTS => {
                    let peers;
                    match app_peers {
                        None => 'a : {
                            for menu in app_state.stack.iter_mut() {
                                if let Menu::PeerList(list) = &mut menu.0 {
                                    peers = list;
                                    break 'a;
                                }
                            }
                            panic!("No peer list found!")
                        }
                        Some(list) => {
                            peers = list;
                        }
                    }
                    let clients : ClientList = postcard::from_bytes(packet.data.as_slice()).unwrap();
                    peers.items = clients.clients;
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

async fn handle_input<'a>(inpt: Input, writer: &mut Arc<Async<TcpStream>>, app: &mut App<'a>) {
    if inpt.key == Key::Char('c') && inpt.ctrl == true {
        graceful_cleanup(&mut app.terminal);
        std::process::exit(0);
    }
    match app.state.focus() {
        Menu::MessageInput(_) => {
            messages_input(inpt, writer, app).await;
        }
        Menu::ImageView => {
            images_input(inpt, &mut app.zoom_x, &mut app.zoom_y, &mut app.state).await;
        }
        Menu::MessageList(_) => {
            messages_list_input(inpt, app).await;
        }
        Menu::PeerList(_) => {
            peer_list_input(inpt, app).await;
        }
        Menu::FileBrowser(_) => {
            file_browser_input(inpt, writer, app).await;
        }
        Menu::Popup(_) => {
            popup_input(inpt, app).await;
        }
    }
}

//todo: does this need to be async?
async fn popup_input(inpt: Input, app: &mut App<'_>) {
    match inpt {
        _ if inpt == SHIFT_BACK => {
            app.state.shift_back();
        }
        _ => {}
    }
}

//todo: does this need to be async?
async fn file_browser_input(inpt: Input, writer: &mut Arc<Async<TcpStream>>, app: &mut App<'_>) {
    match inpt {
        _ if inpt == SHIFT_BACK => {
            app.state.file_browser_mut().unwrap().current_dir_list.unselect();
            app.state.shift_back();
        }
        Input { key: Key::Up, .. } => {
            app.state.file_browser_mut().unwrap().current_dir_list.previous();
        }
        Input { key: Key::Down, .. } => {
            app.state.file_browser_mut().unwrap().current_dir_list.next();
        }
        Input { key: Key::Left, .. } => {
            let current_dir = app.state.file_browser_mut().unwrap().current_dir.clone();
            if Some(Path::new("")) == current_dir.parent() || None == current_dir.parent() {
                return;
            }
            else {
                let _u = std::mem::replace(&mut app.state.file_browser_mut().unwrap().current_dir, PathBuf::from(current_dir.parent().unwrap()));
                app.state.file_browser_mut().unwrap().current_dir_list = StatefulList::with_items(
                    fs::read_dir(app.state.file_browser_mut().unwrap().current_dir.clone()).unwrap().into_iter().map(|entry| {
                        entry.unwrap().path()
                    }).collect());
                app.state.file_browser_mut().unwrap().current_dir_list.state.select(Some(0));
            }
        }
        Input { key: Key::Right, .. } => {
            let selected = app.state.file_browser_mut().unwrap().current_dir_list.current().clone();
            if selected.is_dir() {
                if let Err(_) = fs::read_dir(selected.clone().clone()) {
                    app.state.shift_state(Menu::Popup(
                        Popup {
                            title: "".to_string(),
                            content: "You don't have permission to access this directory!".to_string(),
                        }
                    ), |_, _| {});
                    return;
                }
                //let current = app.state.file_browser_mut().unwrap().current_dir.clone();
                let _u = std::mem::replace(&mut app.state.file_browser_mut().unwrap().current_dir, selected);
                app.state.file_browser_mut().unwrap().current_dir_list = StatefulList::with_items(
                    fs::read_dir(app.state.file_browser_mut().unwrap().current_dir.clone()).unwrap().into_iter().map(|entry| {
                        entry.unwrap().path()
                    }).collect());
                app.state.file_browser_mut().unwrap().current_dir_list.state.select(Some(0));
            }
        }
        Input { key: Key::Enter, .. } => {
            let path = app.state.file_browser_mut().unwrap().current_dir_list.current().as_path();
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
            app.state.shift_back();
        }
        Input { .. } => {}
    }
}

//todo: does this need to be async?
async fn peer_list_input(inpt: Input, app: &mut App<'_>) {
    match inpt {
        _ if inpt == SHIFT_BACK => {
            app.peers_mut().unselect();
            let _ = std::mem::replace(&mut app.peers, Some(
                if let Menu::PeerList(list) = app.state.shift_back() { list } else { panic!() }
            ));
        }
        Input { key: Key::Up, .. } => {
            app.peers_mut().previous();
        }
        Input { key: Key::Down, .. } => {
            app.peers_mut().next();
        }
        Input { .. } => {}
    }
}

//todo: does this need to be async?
async fn messages_list_input(inpt: Input, app: & mut App<'_>) {
    match inpt {
        _ if inpt == SHIFT_BACK => {
            app.messages_mut().unselect();
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
                    app.state.shift_state(Menu::ImageView, |_, _| {});
                }
                MessageType::CLIENTS => {}
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
        _ if inpt == SHIFT_BACK => {
            app_state.shift_back();
        }
        _input => {

        }
    }
}

async fn messages_input(inpt: Input, writer: &mut Arc<Async<TcpStream>>, app: &mut App<'_>) {
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
                    if let Menu::MessageInput(area) = &mut app.state.stack[0].0 {
                        area.set_block(Block::default()
                            .borders(Borders::ALL)
                            .title("Input")
                            .style(Style::default()));
                    }
                    /*let previous_menu = StatefulList::with_items(fs::read_dir(home::home_dir().unwrap()).unwrap().into_iter().map(|entry| {
                        entry.unwrap().path()
                    }).collect());*/
                    let mut current_path = home::home_dir().unwrap();
                    current_path.push("Documents");
                    let mut current_menu = StatefulList::with_items(fs::read_dir(current_path.clone()).unwrap().into_iter().map(|entry| {
                        entry.unwrap().path()
                    }).collect());
                    current_menu.state.select(Some(0));
                    app.state.shift_state(Menu::FileBrowser(FileBrowser {
                        current_dir_list: current_menu,
                        current_dir: current_path,
                    }), |_, _| {});
                    /*let path = Path::new(input_lines[0].split(' ').collect::<Vec<&str>>()[1]);
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
                    break 'a;*/
                }
                "/exit" => {
                    graceful_cleanup(&mut app.terminal);
                    std::process::exit(0);
                }
                _ => {
                    let packet = Packet {
                        src: PktSource::UNDEFINED,
                        message_type: MessageType::STRING,
                        data: Vec::from(input_lines.join("\n"))
                    };
                    send_with_header(writer, packet).await.unwrap();
                }
            }
        },
        _ if inpt == SHIFT_TO_MESSAGES_LIST => {
            app.messages_mut().state.select(Some(0));
            {
                if let Menu::MessageInput(area) = &mut app.state.stack[0].0 {
                    area.set_block(Block::default()
                        .borders(Borders::ALL)
                        .title("Input")
                        .style(Style::default()));
                }
            }
            let message_list = std::mem::replace(&mut app.messages, None);
            app.state.shift_state(Menu::MessageList(message_list.unwrap()), |_menu, state| {
                if let Menu::MessageInput(area) = &mut state.stack[0].0 {
                    area.set_block(Block::default()
                        .borders(Borders::ALL)
                        .title("Input")
                        .style(Style::default()
                            .fg(Color::Blue)));
                };
            });
        },
        _ if inpt == SHIFT_TO_PEERS => {
            app.peers_mut().state.select(Some(0));
            {
                if let Menu::MessageInput(area) = &mut app.state.stack[0].0 {
                    area.set_block(Block::default()
                        .borders(Borders::ALL)
                        .title("Input")
                        .style(Style::default()));
                }
            }
            let peers_list = std::mem::replace(&mut app.peers, None);
            app.state.shift_state(Menu::PeerList(peers_list.unwrap()), |_menu, state| {
                if let Menu::MessageInput(area) = &mut state.stack[0].0 {
                    area.set_block(Block::default()
                        .borders(Borders::ALL)
                        .title("Input")
                        .style(Style::default()
                            .fg(Color::Blue)));
                };
            })
        }
        input => {
            if let Menu::MessageInput(area) = &mut app.state.stack[0].0 {
                area.input(input);
            }
        }
    }
}


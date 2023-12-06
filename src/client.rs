//! Client

#![forbid(unsafe_code)]
#![deny(clippy::all)]
#![warn(clippy::nursery, clippy::cargo)]
#![allow(clippy::needless_return)]
#![feature(let_chains)]
#![warn(clippy::pedantic)]

#[cfg(feature = "protocol_halfblocks")]
use ratatui_image::picker::ProtocolType;
use crossterm::cursor;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use tui_textarea::{Input, Key, TextArea};

use std::net::TcpStream;

use std::error::Error;
use std::{fs, io, sync, thread};
use std::io::StdoutLock;
use std::net::{IpAddr, SocketAddr};
use futures_util::FutureExt;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui_image::picker::{Picker};
use ratatui_image::protocol::{ResizeProtocol};
use ratatui_image::ResizeImage;
use smol::io::{BufReader};
use std::panic;
use core::time::Duration;
use std::fmt::{Debug, Formatter};
use std::ops::{Deref};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvError, Sender};
use std::sync::{Mutex};
use std::thread::{JoinHandle};
use crossterm::execute;
use file_format::FileFormat;
use ratatui::style::{Color, Modifier, Style};
use smol::Async;

use crate::common::{MessageType, Packet, PktSource, read_data, ReadResult, send_with_header, ClientList, Client, ImageData};

pub mod common;

const SEND: Input = Input { key: Key::Char('x'), ctrl: false, alt: false, shift: false };
const IMAGE_ZOOM_IN: Input = Input { key : Key::Char('+'), /*ctrl: true,*/ ctrl: false, alt: false, shift: false };
const IMAGE_ZOOM_OUT: Input = Input { key : Key::Char('-'), /*ctrl: true,*/ ctrl: false, alt: false, shift: false };
const _IMAGE_MOVE_LEFT: Input = Input { key: Key::Left, ctrl: false, alt: false, shift: false };
const _IMAGE_MOVE_RIGHT: Input = Input { key: Key::Right, ctrl: false, alt:false, shift: false };
const _IMAGE_MOVE_UP: Input = Input { key: Key::Up, ctrl: false, alt: false, shift: false };
const _IMAGE_MOVE_DOWN: Input = Input { key: Key::Down, ctrl: false, alt: false, shift: false };
const SHIFT_BACK: Input = Input { key: Key::Char('<'), ctrl: false, alt: false, shift: false };
const MESSAGE_SELECT: Input = Input { key: Key::Enter, ctrl: false, alt: false, shift: false };
const SHIFT_TO_PEERS: Input = Input { key: Key::Char('P'), ctrl: false, alt: false, shift: true };
const SHIFT_TO_MESSAGES_LIST : Input = Input { key: Key::Char('L'), ctrl: false, alt: false, shift: true };
const POLL_TIME: Duration = Duration::from_millis(50);

struct StatefulList<T> {
    state: ListState,
    items: Vec<T>,
    previously_selected: Option<usize>,
    title: String
}

impl<T> StatefulList<T> {
    fn with_items(items: Vec<T>, title: String) -> Self {
        Self {
            state: ListState::default(),
            items,
            previously_selected: None,
            title
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

    fn current(&self) -> Option<&T> {
        return self.items.get(self.state.selected().unwrap())
    }

    fn unselect(&mut self) {
        self.previously_selected = self.state.selected();
        self.state.select(None);
    }
}

impl<'a> StatefulList<Packet> {
    fn to_list(&self, width: usize) -> List<'a> {
        let mut messages_vec : Vec<ListItem> = Vec::new();
        for i in 0..self.items.len() {
            let packet = self.items.get(i).unwrap();
            match &packet.message_type {
                MessageType::STRING => {
                    for sub_string in sub_strings(String::from_utf8(packet.data.clone()).unwrap(), width).iter().rev() {
                        if let Some(index) = self.state.selected() && index == i {
                            messages_vec.push(
                                ListItem::new(format!("{sub_string}\n")).style(Style::default()
                                                                                     .bg(Color::LightGreen)
                                                                                     .add_modifier(Modifier::BOLD),
                                ));
                        }
                        else {
                            messages_vec.push(
                                ListItem::new(format!("{sub_string}\n")).style(Style::default().fg(Color::White).bg(Color::Black)));
                        }
                    }
                }
                MessageType::IMAGE(img) => {
                    messages_vec.push(
                        ListItem::new(format!("Image of type {:?}, {}", img.format, img.name)).style(Style::default().fg(Color::White).bg(Color::Black)));
                }
                MessageType::CLIENTS => {}
            }
        }

        let messages_list = List::new(messages_vec)
            .block(Block::default().title("Messages").borders(Borders::ALL)
                .style(
                    if self.state.selected().is_some() {
                        Style::default()
                    }
                    else {
                        Style::default().fg(Color::DarkGray)
                    }
                ))
            .highlight_style(
                Style::default()
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        messages_list
    }
}

impl<'a> From<&Client> for ListItem<'a> {
    fn from(value: &Client) -> Self {
        ListItem::new(value.addr.to_string()).style(Style::default().fg(Color::White).bg(Color::Black))

    }
}

impl<'a, T> From<&StatefulList<T>> for List<'a>
where for<'b> &'b T: Into<ListItem<'a>>
/* for more information on higher-ranked polymorphism, visit https://doc.rust-lang.org/nomicon/hrtb.html */
{
    fn from(value: &StatefulList<T>) -> Self {
        let mut vec : Vec<ListItem> = Vec::new();
        for item in &value.items {
            vec.push(
                <&T as Into<ListItem>>::into(item)
            );
        }

        let list = List::new(vec)
            .block(Block::default().title(value.title.clone()).borders(Borders::ALL)
                .style(
                    if value.state.selected().is_some() {
                        Style::default()
                    }
                    else {
                        Style::default().fg(Color::DarkGray)
                    }
                ))
            .highlight_style(
                Style::default()
                    .bg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        list
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
    current_dir: PathBuf,
    current_image: sync::Arc<Mutex<Option<Box<dyn ResizeProtocol>>>>,
    image_thread_sender: Sender<Option<PathBuf>>, //when sender is dropped, so will the thread be
    _image_thread: JoinHandle<()> //keep this for good measure
}

impl FileBrowser {
    fn new(current_dir: PathBuf, picker: &Picker) -> Self {
        let mut current_dir_list = StatefulList::with_items(fs::read_dir(current_dir.clone()).unwrap().map(|entry| {
            entry.unwrap().path()
        }).collect(), current_dir.clone().into_os_string().into_string().unwrap());
        current_dir_list.state.select(Some(0));
        let (image_thread_sender, receiver) = channel::<Option<PathBuf>>();
        let current_image = sync::Arc::new(Mutex::new(None));
        let struct_current_image = current_image.clone();
        let mut picker = *picker;
        let image_thread = thread::spawn(move || 'exit: loop {
            match receiver.recv() {
                Ok(None) | Err(RecvError) => {
                    break 'exit;
                }
                Ok(Some(current_file)) => {
                    //todo: add file name back in preview
                    let _name : String = current_file.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap();
                    if current_file.is_file() {
                        let raw = fs::read(current_file).unwrap();
                        let guess = image::guess_format(raw.as_slice());
                        if let Ok(format) = guess {
                            //guess_format does not verify validity of the entire memory - for now, we'll load the image into memory to verify its validity
                            let loaded = image::load_from_memory_with_format(raw.as_slice(), format);
                            if let Ok(dyn_img) = loaded {
                                *current_image.lock().unwrap() = Some(picker.new_resize_protocol(dyn_img));
                                continue 'exit;
                            }
                        }
                    }
                    *current_image.lock().unwrap() = None;
                }
            }
        });
        Self {
            current_dir_list,
            current_dir,
            current_image: struct_current_image,
            image_thread_sender,
            _image_thread: image_thread
        }
    }
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

/* impl<'a> Widget for Menu<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self {
            Menu::MessageInput(textarea) => {
                textarea.widget().render(area, buf);
            }
            Menu::ImageView => {}
            Menu::MessageList(_) => {}
            Menu::PeerList(mut peers) => {
                let mut peers_vec : Vec<ListItem> = Vec::new();
                for peer in peers.items.iter() {
                    peers_vec.push(
                        ListItem::new(peer.addr.to_string()).style(Style::default().fg(Color::White).bg(Color::Black))
                    );
                }

                let peers_list = List::new(peers_vec)
                    .block(Block::default().title("Peers").borders(Borders::ALL)
                        .style(
                            if peers.state.selected().is_some() {
                                Style::default()
                            }
                            else {
                                Style::default().fg(Color::DarkGray)
                            }
                        ))
                    .highlight_style(
                        Style::default()
                            .bg(Color::LightGreen)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol(">> ");

                StatefulWidget::render(peers_list, area, buf, &mut peers.state)
            }
            Menu::FileBrowser(_) => {}
            Menu::Popup(_) => {}
        }
    }
}*/

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

type ShiftBackHandler = Box<dyn FnOnce(&mut Menu, &mut State) + 'static>;

struct StateCallbacks {
    shift_back: ShiftBackHandler, //shifting away from the menu for good
    shift_away: Box<dyn Fn(&mut Menu) + 'static>, //shifting away to another menu
    shift_to: Box<dyn Fn(&mut Menu) + 'static> //shifting back to this menu from another menu
}

/*
            MessageInput
                 |
MessageList <----------> ImageView
*/
struct State<'a> {
    stack: Vec<(Menu<'a>, StateCallbacks)>,
    _cursor_visible: bool
}

impl<'a> State<'a> {
    fn push(&mut self, m: Menu<'a>, callbacks: StateCallbacks) {
        self.stack.push((m, callbacks));
    }

    fn pop(&mut self) -> Option<(Menu<'a>, StateCallbacks)> {
        self.stack.pop()
    }

    fn focus(&self) -> &(Menu<'a>, StateCallbacks) {
        self.stack.last().unwrap()
    }

    fn focus_mut(&mut self) -> &mut(Menu<'a>, StateCallbacks) {
        self.stack.last_mut().unwrap()
    }

    fn shift_state(&mut self, menu: Menu<'a>, callbacks: StateCallbacks) {
        let focus = self.focus_mut();
        (focus.1.shift_away)(&mut focus.0);
        self.push(menu, callbacks);
    }

    fn shift_back(&mut self) -> Menu<'a> {
        let mut returned = self.pop().unwrap();
        (returned.1.shift_back)(&mut returned.0, self);
        let new = self.focus_mut();
        (new.1.shift_to)(&mut new.0);
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
        for menu in &mut self.stack {
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
                for menu in &mut self.state.stack {
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
                for menu in &self.state.stack {
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
                for menu in &mut self.state.stack {
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
        println!("Core dumped");
    }));
    coredump::register_panic_handler().unwrap();
     'restart: loop {
            // Open a TCP stream to the socket address
            let mut stream = None;
            println!("Trying to connect...");
            for _ in 0..5 {
                match smol::block_on(Async::<TcpStream>::connect(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 6000))) {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(_) => {
                        println!("Failed. Retrying..");
                    }
                }
            }

            let stream_ref = async_dup::Arc::new(stream.map_or_else(|| {
                panic!("Could not connect to server after 5 attempts!")
            }, |v| {
                v
            }));
            let mut reader = BufReader::new(stream_ref.clone());
            let mut writer = stream_ref;

            let stdout = io::stdout();
            let mut stdout = stdout.lock();

            //should be initialized before backend
            #[cfg(feature = "protocol_auto")]
                let picker = Picker::from_termios().unwrap();

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
                    .style(Style::default())
            );

            let mut app = App {
                terminal: term,
                image: None,
                picker,
                zoom_x: 35,
                zoom_y: 35,
                state: State {
                    stack: vec![(Menu::MessageInput(textarea), StateCallbacks {
                        shift_back: Box::new(|_, _| { panic!("Shifted state away from base text area!") }),
                        shift_away: Box::new(|menu| {
                            if let Menu::MessageInput(area) = menu {
                                area.set_block(Block::default()
                                    .borders(Borders::ALL)
                                    .title("Input")
                                    .style(Style::default().fg(Color::DarkGray)));
                            }
                        }),
                        shift_to: Box::new(|menu| {
                            if let Menu::MessageInput(area) = menu {
                                area.set_block(Block::default()
                                    .borders(Borders::ALL)
                                    .title("Input")
                                    .style(Style::default()));
                            };
                        }),
                    })],
                    _cursor_visible: true
                },
                messages: Some(StatefulList::with_items(Vec::new(), String::from("Messages"))),
                peers: Some(StatefulList::with_items(Vec::new(), String::from("Peers")))
            };
            loop {
                app.terminal.draw(|f| ui(f, &mut app.messages,
                                         app.zoom_x, app.zoom_y, &mut app.image, &mut app.state, &mut app.peers
                )).unwrap();

                //todo: is this leaking anything?
                match read_data(&mut reader).now_or_never() {
                    None => {}
                    Some(res) => {
                        match res.unwrap() {
                            ReadResult::EMPTY => {}
                            ReadResult::DISCONNECT => {
                                continue 'restart;
                                //panic!("Server Disconnected!");
                            }
                            ReadResult::SUCCESS(bytes_read) => {
                                let packet: Packet = postcard::from_bytes(bytes_read.as_slice()).unwrap();
                                message_recv(packet,
                                             &mut app.messages, &mut app.picker,
                                             &mut app.image, &mut app.state, &mut app.peers);
                            }
                        }
                    }
                }

                let inpt: Input = if crossterm::event::poll(POLL_TIME).unwrap() {
                    crossterm::event::read().unwrap().into()
                }
                else {
                    continue;
                };
                handle_input(inpt, &mut writer, &mut app);
            }
     }
}


fn ui<B: Backend>(f: &mut Frame<B>, app_messages: &mut Option<StatefulList<Packet>>,
                  app_zoom_x: u16, app_zoom_y: u16, app_image: &mut Option<ImageWithData>, app_state: &mut State, app_peers: &mut Option<StatefulList<Client>>)
{

    if let Some(browser) = app_state.file_browser_mut() {
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

        if let Ok(mut mutex) = browser.current_image.try_lock() {
            if let Some(img) = &mut *mutex {
                let img_layout = Block::default().borders(Borders::ALL).title(format!("Image {:?}", browser.current_dir_list.current().unwrap().file_name().unwrap()));
                f.render_stateful_widget(ResizeImage::new(None),
                                         img_layout.inner(right_section[1]),
                                         img);
                f.render_widget(img_layout, right_section[1]);
            }
        }

        if let Some(current_file) = browser.current_dir_list.current() {
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
        else {
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
                    for menu in &mut app_state.stack {
                        if let Menu::MessageList(list) = &mut menu.0 {
                            messages = list;
                            break 'a;
                        }
                    }
                    panic!("No message list found!")
                }
                Some(list) => {
                    messages = list;
                }
            }

            f.render_stateful_widget(messages.to_list(top_section[0].width.into()),top_section[0], &mut messages.state);

            let peers;
            match app_peers {
                None => 'a : {
                    for menu in &mut app_state.stack {
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

            f.render_stateful_widget::<List>(<&StatefulList<Client> as Into<List>>::into(peers), top_section[1], &mut peers.state);

            let app_textarea = if let Menu::MessageInput(x) = &mut app_state.stack[0].0 { x } else { panic!() };
            f.render_widget(app_textarea.widget(), main_layout[1]);

            let _u = app_image.as_mut().map_or(0, |img| {
                if app_state.focus().0 == Menu::ImageView {
                    let img_layout = Block::default().borders(Borders::ALL).title("Image");
                    f.render_stateful_widget(ResizeImage::new(None),
                                             img_layout.inner(centered_rect(f.size(), app_zoom_x, app_zoom_y)),
                                             &mut img.img);
                    f.render_widget(img_layout, centered_rect(f.size(), app_zoom_x, app_zoom_y));
                }
                0
            });
        }
    };
    if let Menu::Popup(p) = &app_state.focus().0 {
        f.render_widget(Paragraph::new(p.content.as_str())
                            .wrap(Wrap {trim: false})
                            .block(Block::default().title(p.title.as_str()).borders(Borders::ALL)), centered_rect(f.size(), 35, 35));
    }

}

fn message_recv(packet: Packet, app_messages: &mut Option<StatefulList<Packet>>,
                app_picker: &mut Picker, app_image: &mut Option<ImageWithData>,
                app_state: &mut State, app_peers: &mut Option<StatefulList<Client>>)
{
    let messages ;
    match app_messages {
        None => 'a : {
            for menu in &mut app_state.stack {
                if let Menu::MessageList(list) = &mut menu.0 {
                    messages = list;
                    break 'a;
                }
            }
            panic!("No message list found!")
        }
        Some(list) => {
            messages = list;
        }
    }
    match packet.message_type.clone() {
        MessageType::STRING => {
            messages.items.push(packet);
        }
        MessageType::IMAGE(ref imgtype) => {
            messages.items.push(packet);
            let dyn_img = image::load_from_memory_with_format(messages.items.last().unwrap().data.as_slice(), imgtype.format).unwrap();
            let image = app_picker.new_resize_protocol(dyn_img.clone());
            *app_image = Option::from(ImageWithData {
                img: image,
                _name: imgtype.name.clone(),
                _height: dyn_img.height(),
                _width: dyn_img.width(),
            });
            app_state.shift_state(Menu::ImageView, StateCallbacks {
                shift_back: Box::new(|_, _| {}),
                shift_away: Box::new(|_| {}),
                shift_to: Box::new(|_| {}),
            });
        }
        MessageType::CLIENTS => {
            let peers;
            match app_peers {
                None => 'a : {
                    for menu in &mut app_state.stack {
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

fn handle_input(inpt: Input, writer: &mut async_dup::Arc<Async<TcpStream>>, app: &mut App<'_>) {
    if inpt.key == Key::Char('c') && inpt.ctrl {
        graceful_cleanup(&mut app.terminal);
        std::process::exit(0);
    }
    match app.state.focus().0 {
        Menu::MessageInput(_) => {
            messages_input(inpt, writer, app);
        }
        Menu::ImageView => {
            images_input(inpt, &mut app.zoom_x, &mut app.zoom_y, &mut app.state);
        }
        Menu::MessageList(_) => {
            messages_list_input(inpt, app);
        }
        Menu::PeerList(_) => {
            peer_list_input(inpt, app);
        }
        Menu::FileBrowser(_) => {
            file_browser_input(inpt, writer, app);
        }
        Menu::Popup(_) => {
            popup_input(inpt, app);
        }
    }
}

fn popup_input(inpt: Input, app: &mut App<'_>) {
    match inpt {
        _ if inpt == SHIFT_BACK => {
            app.state.shift_back();
        }
        _ => {}
    }
}

fn file_browser_input(inpt: Input, writer: &mut async_dup::Arc<Async<TcpStream>>, app: &mut App<'_>) {
    match inpt {
        _ if inpt == SHIFT_BACK => {
            app.state.file_browser_mut().unwrap().current_dir_list.unselect();
            app.state.shift_back();
        }
        Input { key: Key::Up, .. } => {
            let browser = app.state.file_browser_mut().unwrap();
            browser.current_dir_list.previous();
            browser.image_thread_sender.send(Some(browser.current_dir_list.current().unwrap().clone())).unwrap();
        }
        Input { key: Key::Down, .. } => {
            let browser = app.state.file_browser_mut().unwrap();
            browser.current_dir_list.next();
            browser.image_thread_sender.send(Some(browser.current_dir_list.current().unwrap().clone())).unwrap();
        }
        Input { key: Key::Left, .. } => {
            let current_dir = app.state.file_browser_mut().unwrap().current_dir.clone();
            if Some(Path::new("")) == current_dir.parent() || current_dir.parent().is_none() {
                return;
            }
            let _u = std::mem::replace(&mut app.state.file_browser_mut().unwrap().current_dir, PathBuf::from(current_dir.parent().unwrap()));
            app.state.file_browser_mut().unwrap().current_dir_list = StatefulList::with_items(
                fs::read_dir(app.state.file_browser_mut().unwrap().current_dir.clone()).unwrap().map(|entry| {
                    entry.unwrap().path()
                }).collect(), app.state.file_browser_mut().unwrap().current_dir.clone().into_os_string().into_string().unwrap());
            app.state.file_browser_mut().unwrap().current_dir_list.state.select(Some(0));
        }
        Input { key: Key::Right, .. } => {
            let selected = app.state.file_browser_mut().unwrap().current_dir_list.current().unwrap().clone();
            if selected.is_dir() {
                if fs::read_dir(selected.clone()).is_err() {
                    app.state.shift_state(Menu::Popup(
                        Popup {
                            title: String::new(),
                            content: "You don't have permission to access this directory!".to_string(),
                        }
                    ), StateCallbacks {
                        shift_back: Box::new(|_, _| {}),
                        shift_away: Box::new(|_| {}),
                        shift_to: Box::new(|_| {}),
                    });
                    return;
                }
                //let current = app.state.file_browser_mut().unwrap().current_dir.clone();
                let _u = std::mem::replace(&mut app.state.file_browser_mut().unwrap().current_dir, selected);
                app.state.file_browser_mut().unwrap().current_dir_list = StatefulList::with_items(
                    fs::read_dir(app.state.file_browser_mut().unwrap().current_dir.clone()).unwrap().map(|entry| {
                        entry.unwrap().path()
                    }).collect(), app.state.file_browser_mut().unwrap().current_dir.clone().into_os_string().into_string().unwrap());
                app.state.file_browser_mut().unwrap().current_dir_list.state.select(Some(0));
            }
        }
        Input { key: Key::Enter, .. } => {
            let path = app.state.file_browser_mut().unwrap().current_dir_list.current().unwrap().as_path();
            let name : String = path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap();
            if path.is_file() {
                let raw = fs::read(path).unwrap();
                if let Ok(format) = image::guess_format(raw.as_slice()) {
                    //guess_format does not verify validity of the entire memory - for now, we'll load the image into memory to verify its validity
                    let loaded = image::load_from_memory_with_format(raw.as_slice(), format);
                    if let Ok(_dyn_img) = loaded {
                        let packet = Packet {
                            src: PktSource::UNDEFINED,
                            message_type: MessageType::IMAGE(ImageData {
                                format, name}),
                            data: raw,
                        };
                        smol::block_on(send_with_header(writer, packet)).unwrap();
                        app.state.shift_back();
                    }
                    else if let Err(_e) = loaded {
                        app.state.shift_state(Menu::Popup(
                            Popup {
                                title: String::new(),
                                content: "Invalid file format... is this not an image?".to_string(),
                            }
                        ), StateCallbacks {
                            shift_back: Box::new(|_, _| {}),
                            shift_away: Box::new(|_| {}),
                            shift_to: Box::new(|_| {}),
                        });
                    }
                }
            }
        }
        Input { .. } => {}
    }
}

//todo: does this need to be async?
fn peer_list_input(inpt: Input, app: &mut App<'_>) {
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

fn messages_list_input(inpt: Input, app: & mut App<'_>) {
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
            let packet = app.messages().current().unwrap();
            match &packet.message_type {
                MessageType::STRING => {

                }
                MessageType::IMAGE(imagedata) => {
                    let name = imagedata.name.clone();
                    let dyn_img = image::load_from_memory_with_format(packet.data.as_slice(), imagedata.format).unwrap();
                    let image = app.picker.new_resize_protocol(dyn_img.clone());
                    app.image = Option::from(ImageWithData {
                        img: image,
                        _name: name,
                        _height: dyn_img.height(),
                        _width: dyn_img.width(),
                    });
                    app.state.shift_state(Menu::ImageView, StateCallbacks {
                        shift_back: Box::new(|_, _| {}),
                        shift_away: Box::new(|_| {}),
                        shift_to: Box::new(|_| {}),
                    });
                }
                MessageType::CLIENTS => {}
            }
        }
        Input { .. } => {}
    }
}

fn images_input(inpt: Input, app_zoom_x: &mut u16, app_zoom_y: &mut u16, app_state: &mut State<'_>) {
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

fn messages_input(inpt: Input, writer: &mut async_dup::Arc<Async<TcpStream>>, app: &mut App<'_>) {
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
                    let current_path = home::home_dir().unwrap();
                    //current_path.push("Documents");
                    app.state.shift_state(Menu::FileBrowser(FileBrowser::new(current_path, &app.picker)), StateCallbacks {
                        shift_back: Box::new(|_, _| {}),
                        shift_away: Box::new(|_| {}),
                        shift_to: Box::new(|_| {}),
                    });
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
                    smol::block_on(send_with_header(writer, packet)).unwrap();
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
            let message_list = app.messages.take();
            app.state.shift_state(Menu::MessageList(message_list.unwrap()), StateCallbacks {
                shift_back: Box::new(|_, _| {}),
                shift_away: Box::new(|menu| {
                    if let Menu::MessageList(list) = menu {
                        list.unselect();
                    }
                }),
                shift_to: Box::new(|menu| {
                    if let Menu::MessageList(list) = menu {
                        list.state.select(list.previously_selected);
                    }
                }),
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
            let peers_list = app.peers.take();
            app.state.shift_state(Menu::PeerList(peers_list.unwrap()), StateCallbacks {
                shift_to: Box::new(|_| {}),
                shift_away: Box::new(|_| {}),
                shift_back: Box::new(|menu, _state, | {
                    if let Menu::MessageInput(area) = menu {
                        area.set_block(Block::default()
                            .borders(Borders::ALL)
                            .title("Input")
                            .style(Style::default()
                            .fg(Color::Blue)));
                        };
                }),
            });
        }
        input => {
            if let Menu::MessageInput(area) = &mut app.state.stack[0].0 {
                area.input(input);
            }
        }
    }
}

// Splits a string into a vector of strings to appeal to a width (used for word wrap)
fn sub_strings(string: String, split_len: usize) -> Vec<String> {
    let mut subs: Vec<String> = Vec::with_capacity(string.len() / split_len);
    let mut iter = string.chars();
    let mut pos = 0;

    // Case if "" is passed
    if string.is_empty() {
        return vec![String::new()]
    };

    while pos < string.len() {
        let mut len = 0;
        for ch in iter.by_ref().take(split_len) {
            len += ch.len_utf8();
        }
        subs.insert(0, string[pos..pos + len].to_string());
        pos += len;
    }
    subs
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_menu_eq() {
        assert_eq!(Menu::MessageInput(TextArea::default()),
                   Menu::MessageInput(TextArea::default())
        );
    }
    #[test]
    fn test_menu_ne() {
        assert_ne!(Menu::MessageInput(TextArea::default()),
                   Menu::ImageView
        );
    }
}
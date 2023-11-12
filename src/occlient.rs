use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::mpsc::channel,
    thread,
    time::Duration,
};

use anyhow::Result;
use crossterm::{
    cursor::MoveTo,
    event::{poll, read, Event, KeyCode},
    execute,
    style::{Color, Colors, Print, SetColors},
    terminal::{self, disable_raw_mode, enable_raw_mode, Clear, ClearType},
    QueueableCommand,
};
use serde::{Deserialize, Serialize};
use tungstenite::{connect, Message};

use crate::socketevent::{Identity, IdentityState, SocketEvent};

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    servers: Vec<String>,
    username: String,
}

fn input(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

#[derive(Clone, PartialEq, Debug)]
enum MessageState {
    Sent,
    Sending,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Mode {
    Home,
    Normal,
    Message,
    Command,
}

const REFRESH_RATE: u64 = 5;
const FRAME_DELAY: u64 = 1000 / REFRESH_RATE;
const VERSION: &str = "0.0.0";

fn startup() -> Result<Config> {
    let mut config_path = Path::new(&std::env::var("HOME").unwrap()).to_path_buf();
    config_path.push(".config");
    config_path.push("openchat");
    config_path.push("config.toml");

    if !config_path.exists() {
        let parent = config_path.parent().unwrap();
        fs::create_dir_all(parent)?;
        fs::write(&config_path, "servers = []\nusername = \"anonymous\"")?;
    }

    let text = fs::read_to_string(&config_path)?;
    let config = toml::from_str(&text)?;

    Ok(config)
}

pub fn start() -> Result<()> {
    let mut config = startup()?;
    let mut config_path = Path::new(&std::env::var("HOME").unwrap()).to_path_buf();
    config_path.push(".config");
    config_path.push("openchat");
    config_path.push("config.toml");

    println!(" Current username: {:?}", config.username);
    println!("\n 1) Connect to custom URL");
    println!(" 2) Add new server");
    for (i, url) in config.servers.iter().enumerate() {
        println!(" {}) {:?}", i + 3, url);
    }
    let choice = input(" > ")?.parse::<usize>()?;
    let ip = if choice == 1 {
        input("Enter the server URL: ")?
    } else if choice == 2 {
        let url = input("Enter the server URL: ")?;
        config.servers.push(url.clone());
        fs::write(&config_path, toml::to_string(&config)?)?;
        url
    } else {
        config.servers[choice - 3].clone()
    };

    let (mut socket, _) = connect(&ip)?;
    println!("[INFO] Connected successfully");

    let (sender, receiver) = channel();
    let (res_sender, res_receiver) = channel();

    socket.send(Message::Binary(bincode::serialize(
        &SocketEvent::ProvideIdentity(Identity {
            username: config.username.clone(),
        }),
    )?))?;
    let response = bincode::deserialize(&socket.read()?.into_data())?;
    if let SocketEvent::IdentityReceived = response {
    } else {
        panic!("Unexpected response from socket: {:?}", response)
    }

    thread::spawn(move || -> Result<()> {
        loop {
            if let Ok(msg) = receiver.try_recv() {
                socket.send(Message::Binary(bincode::serialize(&msg)?))?;
                bincode::deserialize(&socket.read()?.into_data())?;
            }

            socket.send(Message::Binary(bincode::serialize(
                &SocketEvent::RequestMessages,
            )?))?;
            let response = bincode::deserialize(&socket.read()?.into_data())?;
            if let SocketEvent::Messages(ls) = response {
                if !ls.is_empty() {
                    res_sender.send(
                        ls.into_iter()
                            .map(|(a, b)| (MessageState::Sent, a, b))
                            .collect::<Vec<_>>(),
                    )?;
                }
            } else {
                panic!("Unexpected response from socket: {:?}", response)
            }
        }
    });

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, Clear(ClearType::All))?;

    let mut mode = Mode::Normal;
    let mut editor_input = String::from("");
    let mut cursor_position = 0;
    let mut messages = vec![];

    'runtime: loop {
        if let Ok(new_messages) = res_receiver.try_recv() {
            println!("New messages: {:?}", new_messages);
            messages.extend_from_slice(&new_messages);
            messages.retain(|x| x.0 != MessageState::Sending);
        }

        let (width, height) = terminal::size()?;

        // Draw editor
        let editor_text = match mode {
            Mode::Normal => editor_input.clone(),
            Mode::Home => format!("[CHOICE] {}", editor_input),
            Mode::Message => format!("[MSG] {}", editor_input),
            Mode::Command => format!("[CMD] {}", editor_input),
        };
        let text_length = editor_text.len() as u16;
        let lines_needed = (text_length + width - 1) / width;

        stdout
            .queue(MoveTo(0, height - lines_needed - 1))?
            .queue(SetColors(Colors::new(Color::Reset, Color::Reset)))?
            .queue(Print("╸".repeat(width as usize)))?
            .queue(MoveTo(0, height - lines_needed))?;

        for i in 0..lines_needed {
            stdout
                .queue(Print(" ".repeat(width as usize)))?
                .queue(MoveTo(0, height - lines_needed + i))?;
        }

        for (i, chunk) in editor_text
            .chars()
            .collect::<Vec<_>>()
            .chunks(width as usize)
            .enumerate()
        {
            let line = chunk.iter().collect::<String>();
            stdout
                .queue(MoveTo(0, height - lines_needed + i as u16))?
                .queue(Print(&line))?
                .queue(Print(" ".repeat(width as usize - line.len())))?;
        }

        // Draw home screen
        if mode == Mode::Home {
            stdout
                .queue(MoveTo(0, 1))?
                .queue(Print("1) Direct connect to server"))?
                .queue(MoveTo(0, 2))?
                .queue(Print("2) Add new server"))?;
            for (i, url) in config.servers.iter().enumerate() {
                stdout.queue(MoveTo(0, 3 + i as u16))?.queue(Print(format!(
                    "{}) Connect to {}",
                    i + 3,
                    url
                )))?;
            }
        }

        if !messages.is_empty() {
            // Draw messages from bottom to top
            let mut message_index = (messages.len() - 1) as isize;
            let mut line = height - lines_needed - 1;
            while line > 0 && message_index >= 0 {
                let (state, sender, text) = messages[message_index as usize].clone();
                let state = match state {
                    MessageState::Sent => "".to_string(),
                    MessageState::Sending => "[SENDING] ".to_string(),
                };
                let sender = match sender {
                    IdentityState::Unknown => "Unknown user:".to_string(),
                    IdentityState::Known(identity) => format!("<{}>", identity.username),
                };
                let text = format!("{}{} {}", state, sender, text);
                let lines_needed = (text.len() as u16 + width - 1) / width;
                line -= lines_needed;

                for (i, chunk) in text
                    .chars()
                    .collect::<Vec<_>>()
                    .chunks(width as usize)
                    .enumerate()
                {
                    let text_line = chunk.iter().collect::<String>();
                    stdout
                        .queue(MoveTo(0, line + i as u16))?
                        .queue(Print(&text_line))?
                        .queue(Print(" ".repeat(width as usize - text_line.len())))?;
                }

                message_index -= 1;
            }
            if line > 0 {
                while line > 0 {
                    stdout
                        .queue(MoveTo(0, line))?
                        .queue(Print(" ".repeat(width as usize)))?;
                    line -= 1;
                }
            }
        }

        // Render top information bar
        let (text, colors) = match mode {
            Mode::Normal => ("  NORMAL ", Colors::new(Color::Black, Color::Yellow)),
            Mode::Message => (" MESSAGE ", Colors::new(Color::Black, Color::Green)),
            Mode::Command => (" COMMAND ", Colors::new(Color::Black, Color::Blue)),
            Mode::Home => ("   HOME  ", Colors::new(Color::Black, Color::Red)),
        };
        let version_text = format!(" openchat v{} ", VERSION);
        let username_text = format!("| username: {} ", config.username);
        stdout
            .queue(MoveTo(0, 0))?
            .queue(SetColors(Colors::new(Color::Black, Color::White)))?
            .queue(Print(" mode "))?
            .queue(SetColors(colors))?
            .queue(Print(text))?
            .queue(SetColors(Colors::new(Color::Black, Color::White)))?
            .queue(Print(&version_text))?
            .queue(Print(&username_text))?
            .queue(Print(" ".repeat(
                width as usize - 6 - text.len() - version_text.len() - username_text.len(),
            )))?;

        // Move cursor to correct position in editor
        if mode == Mode::Message || mode == Mode::Command {
            let text_position = editor_text.len() - cursor_position;
            let y = text_position as u16 / width;
            let x = text_position as u16 % width;
            stdout.queue(MoveTo(x, height - lines_needed + y))?;
        }

        stdout.flush()?;

        if poll(Duration::from_millis(FRAME_DELAY))? {
            if let Event::Key(event) = read()? {
                match event.code {
                    KeyCode::Char('q') if mode == Mode::Normal => break 'runtime,

                    // Mode changes
                    KeyCode::Esc if mode == Mode::Command || mode == Mode::Message => {
                        mode = Mode::Normal
                    }
                    KeyCode::Char('m') if mode == Mode::Normal => mode = Mode::Message,
                    KeyCode::Char('c') if mode == Mode::Normal => mode = Mode::Command,

                    // Editor commands
                    KeyCode::Left
                        if mode == Mode::Command || mode == Mode::Message || mode == Mode::Home =>
                    {
                        if cursor_position < editor_input.len() {
                            cursor_position += 1;
                        }
                    }
                    KeyCode::Right
                        if mode == Mode::Command || mode == Mode::Message || mode == Mode::Home =>
                    {
                        if cursor_position > 0 {
                            cursor_position -= 1;
                        }
                    }
                    KeyCode::Backspace
                        if mode == Mode::Command || mode == Mode::Message || mode == Mode::Home =>
                    {
                        editor_input.pop();
                    }
                    KeyCode::Char(chr)
                        if mode == Mode::Command || mode == Mode::Message || mode == Mode::Home =>
                    {
                        editor_input.insert(editor_input.len() - cursor_position, chr);
                    }

                    KeyCode::Enter if mode == Mode::Message => {
                        sender.send(SocketEvent::SendMessage(editor_input.clone()))?;
                        messages.push((
                            MessageState::Sending,
                            IdentityState::Known(Identity {
                                username: config.username.clone(),
                            }),
                            editor_input.clone(),
                        ));
                        editor_input.clear();
                        cursor_position = 0;
                    }
                    KeyCode::Enter if mode == Mode::Command => {
                        let parts = editor_input.split(' ').collect::<Vec<_>>();
                        match parts[0] {
                            "username" => {
                                let new_username = parts[1].to_string();
                                sender.send(SocketEvent::ProvideIdentity(Identity {
                                    username: new_username.clone(),
                                }))?;
                                config.username = new_username;
                                fs::write(&config_path, toml::to_string(&config)?)?;
                            }
                            _ => todo!(),
                        }
                        editor_input.clear();
                        cursor_position = 0;
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    clearscreen::clear()?;
    Ok(())
}

use std::{
    io::{self, stdout, Write},
    sync::mpsc::channel,
    thread,
    time::Duration,
};

use anyhow::Result;
use crossterm::{
    cursor::{MoveRight, MoveTo, MoveUp},
    event::{poll, read, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use tungstenite::{connect, Message};

use crate::socketevent::{Identity, IdentityState, SocketEvent};

#[derive(Debug, Clone, Copy, PartialEq)]
enum MessageState {
    Received,
    Sending,
}

fn input(prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

const HEADER: &str = "   ____  ____  _______   __________  _____  ______
  / __ \\/ __ \\/ ____/ | / / ____/ / / /   |/_  __/
 / / / / /_/ / __/ /  |/ / /   / /_/ / /| | / /   
/ /_/ / ____/ /___/ /|  / /___/ __  / ___ |/ /    
\\____/_/   /_____/_/ |_/\\____/_/ /_/_/  |_/_/    

 v0.0";

pub fn start_client() -> Result<()> {
    let ip = input("Enter the server ip: ")?;
    let (mut socket, _) = connect(&ip)?;
    println!("[INFO] Connected successfully");

    let (sender, receiver) = channel();
    let (res_sender, res_receiver) = channel();

    let username = input("What is your username? ")?;
    socket.send(Message::Binary(bincode::serialize(
        &SocketEvent::ProvideIdentity(Identity {
            username: username.clone(),
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
                socket.send(Message::Binary(bincode::serialize(
                    &SocketEvent::SendMessage(msg),
                )?))?;
                let response = bincode::deserialize(&socket.read()?.into_data())?;
                if let SocketEvent::MessageReceived = response {
                } else {
                    panic!("Unexpected response from socket: {:?}", response)
                }
            }

            socket.send(Message::Binary(bincode::serialize(
                &SocketEvent::RequestMessages,
            )?))?;
            let response = bincode::deserialize(&socket.read()?.into_data())?;
            if let SocketEvent::Messages(ls) = response {
                if !ls.is_empty() {
                    res_sender.send(
                        ls.into_iter()
                            .map(|(a, b)| (MessageState::Received, a, b))
                            .collect::<Vec<_>>(),
                    )?;
                }
            } else {
                panic!("Unexpected response from socket: {:?}", response)
            }
        }
    });

    let mut message_builder = String::new();
    let mut cursor_offset = 0;
    let mut messages: Vec<(MessageState, IdentityState, String)> = vec![];

    enable_raw_mode()?;
    let mut stdout = stdout();

    'runtime: loop {
        if let Ok(new_messages) = res_receiver.try_recv() {
            println!("New messages: {:?}", new_messages);
            messages.extend_from_slice(&new_messages);
            messages.retain(|x| x.0 != MessageState::Sending);
        }

        clearscreen::clear()?;
        let (w, h) = crossterm::terminal::size()?;

        execute!(stdout, MoveTo(0, 12))?;
        for (state, user, text) in &messages {
            let user = match user {
                IdentityState::Unknown => "Unknown".to_string(),
                IdentityState::Known(ident) => format!("<{}>", ident.username),
            };
            let state = match state {
                MessageState::Sending => "[SENDING] ".to_string(),
                MessageState::Received => "".to_string(),
            };
            println!("{}{} : {}\r", state, user, text);
        }

        execute!(stdout, MoveTo(0, 2))?;
        for row in HEADER.split("\n") {
            let row_length = row.len() as u16;
            let remaining = w - row_length;
            println!("{}{}\r", row, " ".repeat(remaining.into()));
        }
        println!(" username: {:?}\r", username);
        println!(" ip: {:?}\r", ip);
        println!("{}\r", "─".repeat(w as usize));

        execute!(stdout, MoveTo(0, h))?;
        println!("\n\r >> {}\r", message_builder);
        execute!(
            stdout,
            MoveUp(1),
            MoveRight(4 + message_builder.len() as u16 - cursor_offset)
        )?;

        if poll(Duration::from_millis(200))? {
            let event = read()?;

            if let Event::Key(keyevent) = event {
                match keyevent.code {
                    KeyCode::Char(chr) => {
                        message_builder.insert(message_builder.len() - cursor_offset as usize, chr);
                    }
                    KeyCode::Enter => {
                        sender.send(message_builder.clone())?;
                        messages.push((
                            MessageState::Sending,
                            IdentityState::Known(Identity {
                                username: username.clone(),
                            }),
                            message_builder.clone(),
                        ));
                        message_builder.clear();
                        cursor_offset = 0;
                    }
                    KeyCode::Backspace => {
                        message_builder.pop();
                    }
                    KeyCode::Left => {
                        if cursor_offset < message_builder.len() as u16 {
                            cursor_offset += 1;
                        }
                    }
                    KeyCode::Right => {
                        if cursor_offset > 0 {
                            cursor_offset -= 1;
                        }
                    }
                    KeyCode::Esc => break 'runtime,
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    Ok(())
}

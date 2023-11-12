use std::{
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

use anyhow::Result;
use log::info;
use tungstenite::{accept, Message};

use crate::socketevent::{IdentityState, SocketEvent};

type Messages = Arc<Mutex<Vec<(IdentityState, String)>>>;

fn handle_connection(stream: TcpStream, messages: Messages) -> Result<()> {
    let mut socket = accept(stream).unwrap();
    let mut known_messages = 0;
    let mut identity = IdentityState::Unknown;

    loop {
        let msg = bincode::deserialize(&socket.read()?.into_data())?;
        match msg {
            SocketEvent::SendMessage(text) => {
                info!("Received message: {:?} from user: {:?}", text, identity);
                messages.lock().unwrap().push((identity.clone(), text));
                socket.send(Message::Binary(bincode::serialize(
                    &SocketEvent::MessageReceived,
                )?))?;
            }
            SocketEvent::RequestMessages => {
                let msg_list = messages.lock().unwrap();
                let to_send = msg_list[known_messages..msg_list.len()].to_vec();
                socket.send(Message::Binary(bincode::serialize(
                    &SocketEvent::Messages(to_send),
                )?))?;
                known_messages = msg_list.len();
            }
            SocketEvent::ProvideIdentity(new_identity) => {
                identity = IdentityState::Known(new_identity);
                socket.send(Message::Binary(bincode::serialize(
                    &SocketEvent::IdentityReceived,
                )?))?;
            }
            _ => todo!(),
        }
    }
}

pub fn start_server(ip: &str) -> Result<()> {
    let messages = Messages::new(Mutex::new(vec![]));

    let server = TcpListener::bind(ip).unwrap();
    info!("Listening on {:?}", ip);

    for stream in server.incoming() {
        let stream = stream?;
        let msg_list = messages.clone();
        let _ = thread::spawn(move || -> Result<()> { handle_connection(stream, msg_list) });
    }
    Ok(())
}

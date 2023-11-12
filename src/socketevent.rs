use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    pub username: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum IdentityState {
    Known(Identity),
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SocketEvent {
    SendMessage(String),
    Messages(Vec<(IdentityState, String)>),
    ProvideIdentity(Identity),
    RequestMessages,
    MessageReceived,
    IdentityReceived,
}

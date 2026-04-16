use serde::Deserialize;

/// Chat message as received from the server. Some fields are not currently
/// used in the desktop client but are part of the server contract.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub text: String,
    pub username: String,
    pub color: String,
    pub registered: bool,
    /// Server-formatted HH:MM in the server's timezone (so every client
    /// sees the same time, independent of its machine TZ).
    pub time: String,
}

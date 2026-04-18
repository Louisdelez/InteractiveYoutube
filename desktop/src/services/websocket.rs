//! Socket.IO client for the Node.js TV server.
//!
//! Event protocol (must match `server/socket/{tv,chat}.js`):
//!
//! Outgoing (desktop → server):
//!   - `tv:switchChannel` (string)        change channel + receive new tv:state
//!   - `tv:requestState`  ()              ask for fresh tv:state immediately
//!   - `chat:message`     ({ text })      send a chat line
//!
//! Incoming (server → desktop):
//!   - `tv:state`         (TvState)       full snapshot (on connect, channel switch, request)
//!   - `tv:sync`          (TvState)       periodic drift correction (every 15 s)
//!   - `chat:batch`       ([msg, …])      batched chat messages (raw array)
//!   - `chat:history`     ([msg, …])      full history on connect / channel switch
//!   - `viewers:count`    ({ count })     viewer count for current channel

use crate::config::SERVER_URL;
use crate::models::tv_state::TvState;
use rust_socketio::{ClientBuilder, Payload};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum ServerEvent {
    /// Authoritative state on connect / channel switch / explicit request.
    TvState(TvState),
    /// Periodic state push every ~15 s; same shape as TvState.
    TvSync(TvState),
    ChatMessage {
        username: String,
        text: String,
        color: String,
        /// Server-formatted HH:MM in the server's timezone. Empty if
        /// the server didn't send one (legacy message).
        time: String,
    },
    /// Full history dump (replaces local buffer). Tuples =
    /// (username, text, color, time_string).
    ChatHistory(Vec<(String, String, String, String)>),
    /// Server wiped all chat history (daily 3am cron) — drop local buffer.
    ChatCleared,
    ViewerCount {
        count: usize,
    },
    /// Total viewers across all channels — broadcast on every channel
    /// presence change, displayed in the topbar.
    ViewerTotal {
        total: usize,
    },
    /// A channel's playlist was updated (new video added, daily refresh).
    /// Planning views should re-fetch.
    PlaylistUpdated {
        channel_id: String,
    },
    /// Server maintenance in 5 minutes.
    MaintenanceWarning,
    /// Server maintenance started (3am cron).
    MaintenanceStart,
    /// Server maintenance ended.
    MaintenanceEnd,
    Connected,
    Disconnected,
}

#[derive(Clone, Debug)]
pub enum ClientCommand {
    SwitchChannel(String),
    RequestState,
    SendChat(String),
    /// Tell the server we're now viewing a different channel so it can
    /// reply with that channel's chat history (per-channel chat).
    ChatChannelChanged(String),
    /// Push the anonymous pseudo + colour generated locally — server
    /// uses these for outgoing chat messages while the user is not
    /// logged in.
    SetAnonymousName { name: String, color: String },
}

pub fn start(events: Sender<ServerEvent>) -> Sender<ClientCommand> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<ClientCommand>();
    thread::spawn(move || run_socket_loop(events, cmd_rx));
    cmd_tx
}

fn run_socket_loop(events: Sender<ServerEvent>, cmd_rx: Receiver<ClientCommand>) {
    loop {
        // Per-connection "alive" flag flipped to false by the
        // `disconnect` handler — lets the inner cmd loop break out
        // and the outer loop reconnect. Without this, mid-session
        // disconnects (server restart) leave us forever in
        // `cmd_rx.recv_timeout`, silently emitting to a dead socket.
        let alive = Arc::new(AtomicBool::new(true));
        let alive_disc = alive.clone();

        let ev_connect = events.clone();
        let ev_state = events.clone();
        let ev_sync = events.clone();
        let ev_chat = events.clone();
        let ev_history = events.clone();
        let ev_viewers = events.clone();
        let ev_total = events.clone();
        let ev_cleared = events.clone();
        let ev_disc = events.clone();

        let build = ClientBuilder::new(SERVER_URL)
            // Note: rust_socketio uses "open" for the connection-established
            // event, not "connect" (which is the Socket.IO namespace event
            // and isn't fired by this client).
            // Note: `connect()` success below also emits Connected — that's
            // the canonical signal. We don't double-emit on the "open"
            // engine.io event (would cause duplicate SwitchChannel sends
            // → duplicate loadfile on reconnect).
            .on("open", move |_, _| {
                let _ = ev_connect; // keep clone alive for the closure type
            })
            .on("tv:state", move |payload, _| {
                if let Payload::Text(values) = payload {
                    if let Some(v) = values.first() {
                        if let Ok(state) = serde_json::from_value::<TvState>(v.clone()) {
                            let _ = ev_state.send(ServerEvent::TvState(state));
                        }
                    }
                }
            })
            .on("tv:sync", move |payload, _| {
                if let Payload::Text(values) = payload {
                    if let Some(v) = values.first() {
                        if let Ok(state) = serde_json::from_value::<TvState>(v.clone()) {
                            let _ = ev_sync.send(ServerEvent::TvSync(state));
                        }
                    }
                }
            })
            .on("chat:batch", move |payload, _| {
                if let Payload::Text(values) = payload {
                    if let Some(v) = values.first() {
                        // Server sends a raw array of messages (no wrapper).
                        if let Some(arr) = v.as_array() {
                            for msg in arr {
                                if let Some(parsed) = parse_chat_message(msg) {
                                    let _ = ev_chat.send(ServerEvent::ChatMessage {
                                        username: parsed.0,
                                        text: parsed.1,
                                        color: parsed.2,
                                        time: parsed.3,
                                    });
                                }
                            }
                        }
                    }
                }
            })
            .on("chat:history", move |payload, _| {
                if let Payload::Text(values) = payload {
                    if let Some(v) = values.first() {
                        if let Some(arr) = v.as_array() {
                            let history: Vec<(String, String, String, String)> = arr
                                .iter()
                                .filter_map(parse_chat_message)
                                .collect();
                            let _ = ev_history.send(ServerEvent::ChatHistory(history));
                        }
                    }
                }
            })
            .on("chat:cleared", move |_payload, _| {
                let _ = ev_cleared.send(ServerEvent::ChatCleared);
            })
            .on("viewers:count", move |payload, _| {
                if let Payload::Text(values) = payload {
                    if let Some(v) = values.first() {
                        if let Some(count) = v.get("count").and_then(|c| c.as_u64()) {
                            let _ = ev_viewers.send(ServerEvent::ViewerCount {
                                count: count as usize,
                            });
                        }
                    }
                }
            })
            .on("viewers:total", move |payload, _| {
                if let Payload::Text(values) = payload {
                    if let Some(v) = values.first() {
                        if let Some(total) = v.get("total").and_then(|c| c.as_u64()) {
                            let _ = ev_total.send(ServerEvent::ViewerTotal {
                                total: total as usize,
                            });
                        }
                    }
                }
            })
            .on("maintenance:warning", {
                let ev = events.clone();
                move |_, _| { let _ = ev.send(ServerEvent::MaintenanceWarning); }
            })
            .on("maintenance:start", {
                let ev = events.clone();
                move |_, _| { let _ = ev.send(ServerEvent::MaintenanceStart); }
            })
            .on("maintenance:end", {
                let ev = events.clone();
                move |_, _| { let _ = ev.send(ServerEvent::MaintenanceEnd); }
            })
            .on("tv:playlistUpdated", {
                let ev = events.clone();
                move |payload, _| {
                    if let Payload::Text(values) = payload {
                        if let Some(v) = values.first() {
                            if let Some(ch) = v.get("channelId").and_then(|c| c.as_str()) {
                                let _ = ev.send(ServerEvent::PlaylistUpdated {
                                    channel_id: ch.to_string(),
                                });
                            }
                        }
                    }
                }
            })
            .on("disconnect", move |_, _| {
                let _ = ev_disc.send(ServerEvent::Disconnected);
                alive_disc.store(false, Ordering::Relaxed);
            });

        // Move events into the connect block so we can also emit Connected on
        // connect() success — rust_socketio's "open" event isn't always
        // delivered against modern socket.io servers, but a successful
        // `.connect()` is itself proof the transport is up.
        let ev_after_connect = events.clone();
        match build.connect() {
            Ok(socket) => {
                let _ = ev_after_connect.send(ServerEvent::Connected);
                // Push the per-session anonymous pseudo + colour to the
                // server immediately on connect — emitting it from the
                // app's event loop (via SetAnonymousName command) was
                // sometimes racing with the first chat history fetch,
                // leaving the user as "Anonyme-xxx" until they sent a
                // 2nd message.
                // Wait briefly for socket.io's namespace handshake to
                // complete — emitting too early after connect() success
                // returns Ok but the packet is dropped silently.
                thread::sleep(Duration::from_millis(150));
                let pseudo = crate::services::pseudo::get_or_create_pseudo();
                let color = crate::services::pseudo::get_or_create_color();
                let _ = socket.emit(
                    "chat:setAnonymousName",
                    serde_json::json!({ "name": pseudo, "color": color }),
                );
                while alive.load(Ordering::Relaxed) {
                    match cmd_rx.recv_timeout(Duration::from_secs(1)) {
                        Ok(ClientCommand::SwitchChannel(ch)) => {
                            let _ = socket.emit("tv:switchChannel", serde_json::Value::String(ch));
                        }
                        Ok(ClientCommand::RequestState) => {
                            let _ = socket.emit("tv:requestState", serde_json::json!({}));
                        }
                        Ok(ClientCommand::SendChat(text)) => {
                            let _ = socket.emit("chat:message", serde_json::json!({ "text": text }));
                        }
                        Ok(ClientCommand::ChatChannelChanged(ch)) => {
                            let _ = socket.emit("chat:channelChanged", serde_json::Value::String(ch));
                        }
                        Ok(ClientCommand::SetAnonymousName { name, color }) => {
                            let _ = socket.emit(
                                "chat:setAnonymousName",
                                serde_json::json!({ "name": name, "color": color }),
                            );
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                // Disconnected — outer loop rebuilds and reconnects.
                let _ = socket.disconnect();
                thread::sleep(Duration::from_secs(1));
            }
            Err(_) => thread::sleep(Duration::from_secs(3)),
        }
    }
}

fn parse_chat_message(v: &Value) -> Option<(String, String, String, String)> {
    let username = v.get("username").and_then(|x| x.as_str())?.to_string();
    let text = v.get("text").and_then(|x| x.as_str())?.to_string();
    let color = v
        .get("color")
        .and_then(|x| x.as_str())
        .unwrap_or("#999")
        .to_string();
    // Server formats HH:MM in its own TZ and ships it — we display as-is.
    // Empty fallback if the server didn't send it (legacy messages).
    let time = v
        .get("time")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Some((username, text, color, time))
}

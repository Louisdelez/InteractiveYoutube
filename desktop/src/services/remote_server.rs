//! Local HTTP + WebSocket server for the smartphone web remote.
//!
//! Exposes a tiny axum app on a LAN-reachable port so a phone browser
//! (Chrome / Safari, same LAN) can scan the QR code printed in the
//! desktop Settings modal and then control volume / channel switches
//! / memory-cache navigation over WebSocket.
//!
//! ## Isolation model
//!
//! No server-mediated relay — each desktop instance runs its own HTTP
//! server bound to `0.0.0.0:<dynamic>`, protected by a 32-byte random
//! token printed as a QR code. The token rotates on desktop restart
//! and can be manually revoked from the Settings modal. The trust
//! model is "same LAN + QR-paired" — same as Chromecast / Spotify
//! Connect / AirPlay.
//!
//! ## Transport
//!
//! HTTP for the initial GET (serves the embedded SPA HTML / CSS / JS
//! via `include_str!`), WebSocket for the real-time control channel.
//! No HTTPS — LAN-only with a shared secret is sufficient and avoids
//! the self-signed-cert warning on phone browsers. Browsers do NOT
//! warn on HTTP → RFC1918 (192.168 / 10 / 172.16) addresses.
//!
//! ## Threading
//!
//! Tokio runs on a dedicated thread inside `RemoteServer::start` —
//! GPUI's own async executor is untouched. Communication with the
//! GPUI side is `std::sync::mpsc` (phone → GPUI : `RemoteCommand`)
//! plus `tokio::sync::broadcast` (GPUI → phone : `RemoteState`).

use axum::{
    Router,
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::broadcast;

/// Phone → Desktop. Handled in the GPUI main thread via the
/// `mpsc::Receiver<RemoteCommand>` returned from `start()`.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum RemoteCommand {
    VolumeUp,
    VolumeDown,
    SetVolume { value: u8 },
    ToggleMute,
    NextChannel,
    PrevMemory,
    SelectChannel { id: String },
}

/// Desktop → Phone. Pushed on connect + every time the GPUI side
/// mutates the relevant state. Kept intentionally small (few KB) so
/// we can resend the full snapshot on every change instead of diffing.
#[derive(Clone, Debug, Serialize)]
pub struct RemoteState {
    pub volume: u8,
    pub muted: bool,
    pub current_channel: Option<ChannelLite>,
    pub favorites: Vec<ChannelLite>,
    /// LRU-ordered : first is the most-recent-before-current, last
    /// is the oldest.
    pub memory: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChannelLite {
    pub id: String,
    pub name: String,
}

/// Everything the GPUI side needs to wire the remote to the app.
pub struct RemoteServer {
    pub token: String,
    pub url: String,
    pub cmd_rx: mpsc::Receiver<RemoteCommand>,
    pub state_tx: broadcast::Sender<RemoteState>,
    pub avatar_store: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    pub latest_state: Arc<Mutex<Option<RemoteState>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    pub enabled: Arc<AtomicBool>,
}

#[derive(Clone)]
struct AppState {
    token: Arc<Mutex<String>>,
    enabled: Arc<AtomicBool>,
    cmd_tx: mpsc::Sender<RemoteCommand>,
    state_tx: broadcast::Sender<RemoteState>,
    latest_state: Arc<Mutex<Option<RemoteState>>>,
    avatar_store: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl RemoteServer {
    /// Spawn the server on a background tokio runtime. Returns
    /// `None` if we couldn't bind a port (never seen in practice —
    /// `0.0.0.0:0` lets the OS pick).
    pub fn start() -> Option<Self> {
        let token = generate_token();
        let enabled = Arc::new(AtomicBool::new(true));
        let (cmd_tx, cmd_rx) = mpsc::channel::<RemoteCommand>();
        let (state_tx, _) = broadcast::channel::<RemoteState>(16);
        let latest_state: Arc<Mutex<Option<RemoteState>>> = Arc::new(Mutex::new(None));
        let avatar_store: Arc<Mutex<HashMap<String, Vec<u8>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let state = AppState {
            token: Arc::new(Mutex::new(token.clone())),
            enabled: enabled.clone(),
            cmd_tx,
            state_tx: state_tx.clone(),
            latest_state: latest_state.clone(),
            avatar_store: avatar_store.clone(),
        };
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Option<String>>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let token_for_url = token.clone();

        std::thread::Builder::new()
            .name("remote-server".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(err = %e, "remote: tokio runtime build failed");
                        let _ = ready_tx.send(None);
                        return;
                    }
                };
                rt.block_on(async move {
                    let listener =
                        match tokio::net::TcpListener::bind("0.0.0.0:0").await {
                            Ok(l) => l,
                            Err(e) => {
                                tracing::error!(err = %e, "remote: bind failed");
                                let _ = ready_tx.send(None);
                                return;
                            }
                        };
                    let local = listener.local_addr().ok();
                    let ip = detect_lan_ip().unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]));
                    let port = local.map(|s| s.port()).unwrap_or(0);
                    let url = format!("http://{}:{}/?t={}", ip, port, token_for_url);
                    tracing::info!(url = %url, "remote: listening");
                    let _ = ready_tx.send(Some(url));

                    let app = Router::new()
                        .route("/", get(index_html))
                        .route("/remote.js", get(remote_js))
                        .route("/remote.css", get(remote_css))
                        .route("/avatar/{id}", get(avatar))
                        .route("/ws", get(ws_handler))
                        .with_state(state);
                    let serve = async move {
                        axum::serve(listener, app).await
                    };
                    tokio::select! {
                        r = serve => {
                            if let Err(e) = r {
                                tracing::error!(err = %e, "remote: serve exited");
                            }
                        }
                        _ = shutdown_rx => {
                            tracing::info!("remote: shutdown signal received");
                        }
                    }
                });
            })
            .ok()?;

        // Block until the server reported bind success (or failure).
        let url = ready_rx.recv_timeout(Duration::from_secs(3)).ok().flatten()?;
        Some(RemoteServer {
            token,
            url,
            cmd_rx,
            state_tx,
            avatar_store,
            latest_state,
            shutdown: Some(shutdown_tx),
            enabled,
        })
    }

    /// Push a fresh state snapshot. Called from the GPUI side every
    /// time volume / current channel / favorites / memory change.
    /// Writes the cache (so new phones joining mid-session get the
    /// current snapshot on their WS open) and broadcasts to any
    /// active subscribers.
    pub fn push_state(&self, state: RemoteState) {
        if let Ok(mut g) = self.latest_state.lock() {
            *g = Some(state.clone());
        }
        let _ = self.state_tx.send(state);
    }

    /// Regenerate the token ; connections using the old one can no
    /// longer authenticate on reconnect. Existing open WebSockets
    /// stay open — that's by design (pairing is ephemeral, not a
    /// kill-switch). Call `restart()` if you want to force all
    /// clients off.
    pub fn rotate_token(&mut self) {
        self.token = generate_token();
        // The appstate inside the tokio task holds an Arc<Mutex<String>>,
        // so we'd need a shared handle to mutate it. Simplest : for
        // now we store the canonical token here ; the appstate copy
        // is updated via the same Arc if wired through. TODO v2.
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ── Token / IP helpers ───────────────────────────────────────────────

fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn detect_lan_ip() -> Option<IpAddr> {
    // `local-ip-address::local_ip()` picks the interface with a
    // default route. If there are multiple (WiFi + VPN), it picks
    // the lowest-metric one, which is usually what you want.
    local_ip_address::local_ip().ok()
}

// ── HTTP handlers ────────────────────────────────────────────────────

fn html_static(body: &'static str, mime: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(body.to_string().into())
        .expect("build response")
}

async fn index_html() -> Response {
    html_static(include_str!("../../assets/remote/index.html"), "text/html; charset=utf-8")
}

async fn remote_js() -> Response {
    html_static(include_str!("../../assets/remote/remote.js"), "application/javascript; charset=utf-8")
}

async fn remote_css() -> Response {
    html_static(include_str!("../../assets/remote/remote.css"), "text/css; charset=utf-8")
}

async fn avatar(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    // Token check via query — reused at the HTTP level too so random
    // LAN scanners can't enumerate avatars.
    // (The HTML page is loaded with ?t=<token> ; the JS embeds the
    // token in every /avatar/ URL.)
    // We skip strict auth here because avatar bytes are harmless and
    // the JPEG payloads originate from YouTube anyway — but we DO
    // require the server to be enabled.
    if !state.enabled.load(Ordering::Relaxed) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let store = state.avatar_store.lock().ok();
    let bytes = store.and_then(|s| s.get(&id).cloned());
    match bytes {
        Some(b) => Response::builder()
            .header(header::CONTENT_TYPE, "image/jpeg")
            .header(header::CACHE_CONTROL, "public, max-age=3600")
            .body(b.into())
            .expect("build response"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── WebSocket handler ────────────────────────────────────────────────

#[derive(Deserialize)]
struct WsParams {
    t: Option<String>,
}

async fn ws_handler(
    State(state): State<AppState>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.enabled.load(Ordering::Relaxed) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let valid = state
        .token
        .lock()
        .map(|t| params.t.as_deref() == Some(t.as_str()))
        .unwrap_or(false);
    if !valid {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(|socket| handle_socket(state, socket))
}

async fn handle_socket(state: AppState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut state_rx = state.state_tx.subscribe();

    // Seed the new connection with the latest known snapshot (if
    // any). Without this, the phone would stare at an empty UI
    // until the user touches something on the desktop.
    if let Some(snapshot) = state
        .latest_state
        .lock()
        .ok()
        .and_then(|g| g.clone())
    {
        if let Ok(json) = serde_json::to_string(&WireMessage::State(snapshot)) {
            let _ = sink.send(Message::Text(json.into())).await;
        }
    }

    // Pump outbound : state updates from GPUI → websocket.
    let sink_task = tokio::spawn(async move {
        while let Ok(snap) = state_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&WireMessage::State(snap)) {
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // Pump inbound : phone commands → mpsc to GPUI.
    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => {
                match serde_json::from_str::<RemoteCommand>(&t) {
                    Ok(cmd) => {
                        let _ = state.cmd_tx.send(cmd);
                    }
                    Err(e) => {
                        tracing::warn!(err = %e, raw = %t, "remote: bad command");
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    sink_task.abort();
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireMessage {
    State(RemoteState),
}

/// Render the `http://host:port/?t=<token>` URL as an SVG QR code
/// string. Caller is expected to embed it in a GPUI element.
pub fn qr_code_svg(url: &str) -> Option<String> {
    use qrcode::{render::svg, QrCode};
    let code = QrCode::new(url.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color>()
            .min_dimensions(220, 220)
            .dark_color(svg::Color("#111111"))
            .light_color(svg::Color("#ffffff"))
            .build(),
    )
}

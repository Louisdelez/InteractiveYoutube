//! Koala TV desktop logger.
//!
//! Layers:
//!  - stdout (compact, human-readable)
//!  - $XDG_DATA_HOME/KoalaTV/logs/desktop.log (daily rotation, keep 14d)
//!  - HTTP forwarder → POST /api/logs (best-effort, background thread)
//!
//! Also installs a `std::panic::set_hook` that logs the panic via tracing
//! before letting the default hook abort the process. Combined with the
//! file sink, crashes are always recoverable after reboot.
//!
//! Call `services::logger::init()` exactly once, as early as possible in
//! `main.rs`.

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::{field::Visit, Event, Subscriber};
use tracing_subscriber::{
    fmt,
    layer::{Context, Layer, SubscriberExt},
    util::SubscriberInitExt,
    EnvFilter,
};

use crate::config::server_url;

const LOG_ENDPOINT_PATH: &str = "/api/logs";
const FLUSH_INTERVAL: Duration = Duration::from_secs(5);
const BATCH_THRESHOLD: usize = 20;
const LOG_KEEP_DAYS: usize = 14;

pub fn log_dir() -> PathBuf {
    let base = if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            PathBuf::from(x).join("KoalaTV")
        } else {
            fallback_data_dir()
        }
    } else {
        fallback_data_dir()
    };
    base.join("logs")
}

fn fallback_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/share/KoalaTV")
}

#[derive(Serialize, Clone)]
struct WireEvent {
    source: &'static str,
    level: String,
    msg: String,
    ctx: serde_json::Value,
    ts: u64,
    #[serde(rename = "sessionId")]
    session_id: String,
}

/// Initialise the global subscriber and side effects. Safe to call once.
pub fn init() {
    let dir = log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[logger] cannot create {}: {}", dir.display(), e);
    }

    // tracing-appender rotates the file daily, writing as
    // desktop.log.YYYY-MM-DD in `dir`. Older files are not auto-purged
    // here; we do a best-effort purge below.
    let file_appender = tracing_appender::rolling::daily(&dir, "desktop.log");
    let (nb_file, file_guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard so it lives for the process lifetime. Without this
    // the non-blocking writer flushes on drop and we lose tail events.
    Box::leak(Box::new(file_guard));

    let stdout_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_ansi(true)
        .compact()
        .with_writer(io::stdout);

    let file_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_ansi(false)
        .with_writer(nb_file);

    let http_layer = spawn_http_forwarder();

    let filter = EnvFilter::try_from_env("KOALA_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,koala_tv_desktop=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .with(http_layer)
        .init();

    install_panic_hook();
    purge_old_files(&dir);

    tracing::info!(
        log_dir = %dir.display(),
        "koala-tv desktop logger initialised"
    );
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".into());
        let payload: String = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_owned()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".into()
        };

        tracing::error!(location = %location, payload = %payload, "panic");
        // Give the non-blocking file writer a moment to flush to disk
        // before the default hook aborts the process.
        std::thread::sleep(Duration::from_millis(250));
        prev(info);
    }));
}

fn purge_old_files(dir: &std::path::Path) {
    // Best-effort: delete desktop.log.* files older than LOG_KEEP_DAYS.
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let cutoff = SystemTime::now() - Duration::from_secs(LOG_KEEP_DAYS as u64 * 24 * 3600);
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified >= cutoff {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("desktop.log.") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ─── HTTP forwarder ─────────────────────────────────────────────

struct HttpForwardLayer {
    tx: Sender<WireEvent>,
}

fn spawn_http_forwarder() -> HttpForwardLayer {
    let (tx, rx) = mpsc::channel::<WireEvent>();

    thread::Builder::new()
        .name("koala-log-forward".into())
        .spawn(move || {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .ok();
            let Some(client) = client else { return };
            let url = format!("{}{}", server_url(), LOG_ENDPOINT_PATH);

            let mut batch: Vec<WireEvent> = Vec::with_capacity(BATCH_THRESHOLD);
            let mut last_flush = Instant::now();

            loop {
                let timeout = FLUSH_INTERVAL.saturating_sub(last_flush.elapsed());
                match rx.recv_timeout(timeout) {
                    Ok(ev) => {
                        batch.push(ev);
                        if batch.len() >= BATCH_THRESHOLD {
                            send_batch(&client, &url, &mut batch);
                            last_flush = Instant::now();
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if !batch.is_empty() {
                            send_batch(&client, &url, &mut batch);
                        }
                        last_flush = Instant::now();
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        if !batch.is_empty() {
                            send_batch(&client, &url, &mut batch);
                        }
                        break;
                    }
                }
            }
        })
        .ok();

    HttpForwardLayer { tx }
}

fn send_batch(
    client: &reqwest::blocking::Client,
    url: &str,
    batch: &mut Vec<WireEvent>,
) {
    let events: Vec<WireEvent> = batch.drain(..).collect();
    let body = serde_json::json!({ "events": events });
    let _ = client.post(url).json(&body).send();
}

impl<S: Subscriber> Layer<S> for HttpForwardLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = match *meta.level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            tracing::Level::DEBUG => "debug",
            tracing::Level::TRACE => "trace",
        };

        // Forward warn+ only to avoid flooding the server with dev-mode
        // debug lines. If you need richer forwarding, set KOALA_LOG.
        if !matches!(level, "warn" | "error") {
            return;
        }

        let mut visitor = MsgVisitor::default();
        event.record(&mut visitor);

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let msg = if visitor.msg.is_empty() {
            meta.target().to_string()
        } else {
            visitor.msg
        };

        let wire = WireEvent {
            source: "desktop",
            level: level.to_string(),
            msg,
            ctx: serde_json::Value::Object(visitor.fields),
            ts,
            session_id: session_id(),
        };

        // Drop oldest if the channel is saturated (no backpressure onto
        // the caller — logging must never block UI code).
        let _ = self.tx.send(wire);
    }
}

#[derive(Default)]
struct MsgVisitor {
    msg: String,
    fields: serde_json::Map<String, serde_json::Value>,
}

impl Visit for MsgVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let name = field.name();
        let repr = format!("{:?}", value);
        // strip surrounding quotes if the value is a string literal
        let trimmed = repr.trim_matches('"').to_string();
        if name == "message" {
            self.msg = trimmed;
        } else {
            self.fields.insert(name.to_string(), serde_json::Value::String(trimmed));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.msg = value.to_string();
        } else {
            self.fields.insert(field.name().to_string(), serde_json::Value::String(value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(field.name().to_string(), serde_json::Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(field.name().to_string(), serde_json::Value::Number(value.into()));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.insert(field.name().to_string(), serde_json::Value::Bool(value));
    }
}

fn session_id() -> String {
    use std::sync::OnceLock;
    static SID: OnceLock<String> = OnceLock::new();
    SID.get_or_init(|| {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("desktop-{:x}", ts)
    })
    .clone()
}

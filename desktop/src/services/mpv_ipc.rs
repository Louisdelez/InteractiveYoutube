//! External-mpv IPC client.
//!
//! Phase 1 of the libmpv2 → subprocess refactor (ADR: architecture
//! option A). Gives us a `MpvIpcClient` whose public surface is a
//! tight subset of `libmpv2::Mpv` — the actual methods this codebase
//! calls. The implementation spawns mpv with
//! `--input-ipc-server=<unix-sock>` and talks JSON over the socket.
//!
//! Why this approach:
//!
//!  * IPC round-trip measured at 0.038 ms avg, 0.098 ms p99 on our
//!    hardware (see `bin/mpv_ipc_poc.rs`) — not a bottleneck.
//!  * Desktop thread count drops by ~80/mpv (libmpv2 brings the whole
//!    decoder + demuxer + VO thread pool into our process).
//!  * mpv crashes stop killing the UI — we can respawn transparently.
//!
//! Protocol reference: <https://mpv.io/manual/stable/#json-ipc>.
//! Each line over the socket is either:
//!   - a command request: `{"command":["X",...], "request_id":N}`
//!   - a command reply:   `{"request_id":N, "error":"success", "data":...}`
//!   - an async event:    `{"event":"video-reconfig", ...}`
//!
//! Our wrapper owns a background reader thread that demultiplexes
//! these three classes into:
//!   - a `pending` map (request_id → oneshot reply channel)
//!   - an event channel consumed by `wait_event(...)`

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A subset of mpv's async events. We only model the ones Koala TV
/// consumes; everything else collapses to `Other(name)` so the
/// `wait_event` loop can still drain the queue without getting stuck.
#[derive(Clone, Debug)]
pub enum MpvEvent {
    /// First frame of a newly loaded file is decoded and the video
    /// output is configured — "swap-up safe" signal. Sacred: see
    /// CLAUDE.md / feedback_dual_mpv_pattern on why `PlaybackRestart`
    /// is not a valid substitute.
    VideoReconfig,
    /// Fires after a seek or loadfile when playback actually resumes.
    /// Arrives earlier than `VideoReconfig`; do not reveal a video on
    /// this alone.
    PlaybackRestart,
    /// A new file has been loaded (yt-dlp resolved, demuxer ready,
    /// first packet queued).
    FileLoaded,
    /// The current file ended (reason in `data` — not yet surfaced).
    EndFile,
    /// Playback started for a file.
    StartFile,
    /// Any other named event (e.g. "seek", "audio-reconfig", etc.).
    Other(String),
}

/// External mpv instance controlled over the JSON IPC socket.
///
/// `Clone` is cheap (wraps an `Arc`); multiple handles to the same
/// mpv can coexist. All sends are serialised behind the internal
/// write mutex.
#[derive(Clone)]
pub struct MpvIpcClient {
    inner: Arc<Inner>,
}

struct Inner {
    child: Mutex<Option<Child>>,
    socket_path: PathBuf,
    writer: Mutex<UnixStream>,
    next_request_id: AtomicU32,
    /// Pending command replies indexed by request_id. The reader
    /// thread pops each match and sends the reply into the oneshot
    /// mpsc channel held here.
    pending: Arc<Mutex<HashMap<u32, mpsc::Sender<Value>>>>,
    /// Async event queue consumed by `wait_event`.
    events: Mutex<mpsc::Receiver<MpvEvent>>,
}

impl MpvIpcClient {
    /// Spawn a fresh mpv subprocess with the given command-line flags
    /// (e.g. `"--wid=12345"`, `"--vo=gpu-next"`). The IPC socket path
    /// is generated per-instance so multiple clients don't collide.
    ///
    /// Runtime properties (`set_property` etc.) are settable after
    /// this returns — only startup-mandatory options like `wid`,
    /// `force-window`, `vo`, `ao` need to be in `flags`.
    pub fn spawn(flags: &[&str]) -> Result<Self, String> {
        let socket_path = unique_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        let mut cmd = Command::new("mpv");
        cmd.arg("--idle=yes")
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .arg("--no-terminal")
            .arg("--no-input-default-bindings");
        for f in flags {
            cmd.arg(f);
        }
        let child = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn mpv: {e}"))?;

        // Wait for the socket file (mpv creates it a few ms in).
        let deadline = Instant::now() + Duration::from_secs(3);
        while !socket_path.exists() {
            if Instant::now() > deadline {
                return Err("mpv IPC socket didn't appear within 3s".into());
            }
            thread::sleep(Duration::from_millis(10));
        }

        let stream = UnixStream::connect(&socket_path)
            .map_err(|e| format!("connect IPC socket: {e}"))?;
        let reader_half = stream
            .try_clone()
            .map_err(|e| format!("clone socket: {e}"))?;
        let writer_half = stream;

        let pending: Arc<Mutex<HashMap<u32, mpsc::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, event_rx) = mpsc::channel::<MpvEvent>();
        let pending_reader = pending.clone();
        thread::spawn(move || reader_loop(reader_half, pending_reader, event_tx));

        Ok(MpvIpcClient {
            inner: Arc::new(Inner {
                child: Mutex::new(Some(child)),
                socket_path,
                writer: Mutex::new(writer_half),
                next_request_id: AtomicU32::new(1),
                pending,
                events: Mutex::new(event_rx),
            }),
        })
    }

    /// Set a property. Accepts any `Serialize` value; the JSON IPC
    /// format is the same type-system as mpv's native properties
    /// (string for strings, number for i64/f64, bool for yes/no, etc.).
    pub fn set_property<T: Serialize>(&self, name: &str, value: T) -> Result<(), String> {
        let v = serde_json::to_value(value).map_err(|e| format!("serialize: {e}"))?;
        self.request_blocking("set_property", &[Value::String(name.to_string()), v])?;
        Ok(())
    }

    /// Fetch a property and deserialise into `T`. Returns the IPC
    /// error string on failure (mpv rejects unknown properties with
    /// `"property not found"`).
    pub fn get_property<T: DeserializeOwned>(&self, name: &str) -> Result<T, String> {
        let data = self.request_blocking("get_property", &[Value::String(name.to_string())])?;
        serde_json::from_value(data).map_err(|e| format!("deserialize {name}: {e}"))
    }

    /// Run an mpv command. `args` become the JSON command array's
    /// tail (`["cmd", arg0, arg1, ...]`). Blocks until mpv ACKs the
    /// command.
    pub fn command(&self, name: &str, args: &[&str]) -> Result<(), String> {
        let mut payload = Vec::with_capacity(args.len());
        for a in args {
            payload.push(Value::String(a.to_string()));
        }
        self.request_blocking(name, &payload)?;
        Ok(())
    }

    /// Pop the next async event, waiting up to `timeout_secs`. `0.0`
    /// means non-blocking. Returns `None` on timeout so the caller's
    /// drain loops stay idiomatic.
    pub fn wait_event(&self, timeout_secs: f64) -> Option<MpvEvent> {
        let d = if timeout_secs <= 0.0 {
            Duration::from_millis(0)
        } else {
            Duration::from_secs_f64(timeout_secs)
        };
        let rx = self.inner.events.lock().ok()?;
        if timeout_secs <= 0.0 {
            rx.try_recv().ok()
        } else {
            rx.recv_timeout(d).ok()
        }
    }

    /// Graceful shutdown: send `quit`, then wait for the child.
    /// Called from `Drop` too.
    pub fn shutdown(&self) {
        let _ = self.send_raw(&json!({"command": ["quit"]}));
        if let Ok(mut slot) = self.inner.child.lock() {
            if let Some(mut ch) = slot.take() {
                // Short grace period, then SIGKILL if still running.
                for _ in 0..20 {
                    match ch.try_wait() {
                        Ok(Some(_)) => break,
                        Ok(None) => thread::sleep(Duration::from_millis(25)),
                        Err(_) => break,
                    }
                }
                let _ = ch.kill();
                let _ = ch.wait();
            }
        }
        let _ = std::fs::remove_file(&self.inner.socket_path);
    }

    // ── internals ─────────────────────────────────────────────────

    fn request_blocking(&self, name: &str, tail: &[Value]) -> Result<Value, String> {
        let id = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed);
        let mut cmd_array = vec![Value::String(name.to_string())];
        cmd_array.extend_from_slice(tail);
        let req = json!({
            "command": cmd_array,
            "request_id": id
        });
        let (tx, rx) = mpsc::channel::<Value>();
        {
            let mut p = self.inner.pending.lock().map_err(|_| "poisoned pending".to_string())?;
            p.insert(id, tx);
        }
        self.send_raw(&req)?;
        let reply = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| format!("ipc timeout on {name}"))?;
        // Remove in case the reader already popped; idempotent.
        if let Ok(mut p) = self.inner.pending.lock() {
            p.remove(&id);
        }
        if reply.get("error").and_then(|e| e.as_str()) != Some("success") {
            let err = reply
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            return Err(format!("mpv error on {name}: {err}"));
        }
        Ok(reply.get("data").cloned().unwrap_or(Value::Null))
    }

    fn send_raw(&self, cmd: &Value) -> Result<(), String> {
        let mut w = self
            .inner
            .writer
            .lock()
            .map_err(|_| "poisoned writer".to_string())?;
        writeln!(w, "{}", cmd).map_err(|e| format!("write: {e}"))?;
        w.flush().map_err(|e| format!("flush: {e}"))?;
        Ok(())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Last strong handle dropped → tear down mpv.
        if let Some(mut ch) = self.child.lock().ok().and_then(|mut s| s.take()) {
            let _ = ch.kill();
            let _ = ch.wait();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

fn unique_socket_path() -> PathBuf {
    let pid = std::process::id();
    // Nanoseconds from UNIX_EPOCH keep different clients in the same
    // process from colliding (two backup-mpv instances, each with
    // their own socket).
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    PathBuf::from(format!("/tmp/koala-mpv-{}-{}.sock", pid, ns))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ignored by default because it requires `mpv` in PATH. Run with:
    /// `cargo test --release -- --ignored mpv_ipc`.
    #[test]
    #[ignore]
    fn round_trip_latency() {
        let client =
            MpvIpcClient::spawn(&["--vo=null", "--ao=null"]).expect("spawn mpv");
        // Warmup to amortise JSON parser cold start.
        for _ in 0..20 {
            let _: String = client.get_property("mpv-version").expect("get");
        }
        let mut lats = Vec::with_capacity(500);
        for _ in 0..500 {
            let t = Instant::now();
            let _: String = client.get_property("mpv-version").expect("get");
            lats.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let mut sorted = lats.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p99 = sorted[(sorted.len() as f64 * 0.99) as usize];
        println!("p99 round-trip = {:.3} ms", p99);
        assert!(p99 < 3.0, "wrapper round-trip p99 {:.3}ms exceeds 3ms", p99);
        client.shutdown();
    }

    #[test]
    #[ignore]
    fn set_get_round_trip() {
        let client =
            MpvIpcClient::spawn(&["--vo=null", "--ao=null"]).expect("spawn mpv");
        client.set_property("volume", 42i64).expect("set volume");
        let v: f64 = client.get_property("volume").expect("get volume");
        assert!((v - 42.0).abs() < 0.1, "read back {} instead of 42", v);
        client.shutdown();
    }

    #[test]
    #[ignore]
    fn loadfile_video_reconfig_event() {
        let test_file = "/usr/share/help/C/gnome-help/figures/display-dual-monitors.webm";
        if !std::path::Path::new(test_file).exists() {
            eprintln!("SKIP: {} missing", test_file);
            return;
        }
        let client =
            MpvIpcClient::spawn(&["--vo=null", "--ao=null"]).expect("spawn mpv");
        client.command("loadfile", &[test_file]).expect("loadfile");
        let start = Instant::now();
        let mut saw = false;
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(MpvEvent::VideoReconfig) = client.wait_event(0.1) {
                saw = true;
                break;
            }
        }
        assert!(saw, "video-reconfig didn't arrive within 5s");
        client.shutdown();
    }

    #[test]
    #[ignore]
    fn clean_shutdown() {
        let client =
            MpvIpcClient::spawn(&["--vo=null", "--ao=null"]).expect("spawn mpv");
        let _: String = client.get_property("mpv-version").expect("pre-shutdown ping");
        let t = Instant::now();
        client.shutdown();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        assert!(ms < 1000.0, "shutdown took {:.1}ms (>1s)", ms);
    }
}

fn reader_loop(
    stream: UnixStream,
    pending: Arc<Mutex<HashMap<u32, mpsc::Sender<Value>>>>,
    events: mpsc::Sender<MpvEvent>,
) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(v): Result<Value, _> = serde_json::from_str(&line) else {
            continue;
        };
        if let Some(id) = v.get("request_id").and_then(|x| x.as_u64()) {
            // Command reply — dispatch to the pending oneshot.
            let id = id as u32;
            let tx = {
                let mut p = match pending.lock() {
                    Ok(p) => p,
                    Err(_) => break,
                };
                p.remove(&id)
            };
            if let Some(tx) = tx {
                let _ = tx.send(v);
            }
        } else if let Some(name) = v.get("event").and_then(|e| e.as_str()) {
            let ev = match name {
                "video-reconfig" => MpvEvent::VideoReconfig,
                "playback-restart" => MpvEvent::PlaybackRestart,
                "file-loaded" => MpvEvent::FileLoaded,
                "end-file" => MpvEvent::EndFile,
                "start-file" => MpvEvent::StartFile,
                other => MpvEvent::Other(other.to_string()),
            };
            if events.send(ev).is_err() {
                break; // main dropped the client
            }
        }
    }
}

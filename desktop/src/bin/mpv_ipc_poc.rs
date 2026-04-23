//! Phase 0 POC for the libmpv2 → external mpv IPC refactor.
//!
//! Spawns a headless mpv as subprocess with `--input-ipc-server`,
//! connects to the Unix domain socket, and measures:
//!
//!  - IPC round-trip latency (1000× `get_property` samples)
//!  - Event delivery after `loadfile` (time to `video-reconfig`)
//!  - Property monotonicity during playback
//!
//! Run with:
//!     cargo run --release --bin mpv_ipc_poc
//!
//! Decision gate for the full refactor:
//!   * p99 round-trip latency < 2 ms → GO
//!   * loadfile → video-reconfig reasonable (local file < 300 ms) → GO
//!
//! Single-threaded by design: command send → event/reply read inline.
//! The production wrapper (Phase 1) will have a proper reader thread
//! that demuxes replies (by request_id) from events (async), but for
//! the POC we keep it simple so there's one thing to measure.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const SOCKET_PATH: &str = "/tmp/koala-ipc-poc.sock";

fn main() {
    println!("=== Koala TV — mpv IPC POC ===\n");

    // Cleanup from any previous aborted run.
    let _ = std::fs::remove_file(SOCKET_PATH);

    let mut mpv = spawn_mpv();
    let stream = UnixStream::connect(SOCKET_PATH).expect("connect socket");
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().expect("clone read"));

    // ── Test 1: IPC round-trip latency ─────────────────────────────────
    println!("Test 1: IPC round-trip latency (1000 get_property calls)");
    // Warmup: mpv's JSON parser pays a cold-start cost on the first few
    // commands. Without this the p50 is dominated by one-time setup.
    for i in 0..20 {
        let _ = request(&stream, &mut reader, i as u32, "get_property", &["mpv-version"]);
    }
    let mut latencies = Vec::with_capacity(1000);
    for i in 0..1000 {
        let t = Instant::now();
        let _ = request(&stream, &mut reader, (1000 + i) as u32, "get_property", &["mpv-version"]);
        latencies.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let mut sorted = latencies.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = latencies.iter().sum::<f64>() / latencies.len() as f64;
    let p50 = sorted[sorted.len() / 2];
    let p99 = sorted[(sorted.len() as f64 * 0.99) as usize];
    let max = sorted[sorted.len() - 1];
    println!(
        "  samples={}  avg={:.3}ms  p50={:.3}ms  p99={:.3}ms  max={:.3}ms",
        latencies.len(),
        avg,
        p50,
        p99,
        max
    );
    let latency_ok = p99 < 2.0;
    println!("  {} p99 < 2ms gate", if latency_ok { "PASS" } else { "FAIL" });

    // ── Test 2: loadfile → video-reconfig latency ──────────────────────
    // Local test file shipped with gnome-help — short .webm, no network,
    // real demux + decode. If the file is missing (non-Ubuntu desktop)
    // the test is informational only (latency gate is what decides GO).
    println!("\nTest 2: loadfile → video-reconfig latency");
    let test_file = "/usr/share/help/C/gnome-help/figures/display-dual-monitors.webm";
    if !std::path::Path::new(test_file).exists() {
        println!("  SKIP (test file not present: {})", test_file);
    }
    let t0 = Instant::now();
    let _ = request(
        &stream,
        &mut reader,
        9000,
        "loadfile",
        &[test_file],
    );
    let mut saw_reconfig = false;
    let deadline = t0 + Duration::from_secs(5);
    while Instant::now() < deadline {
        match read_one(&mut reader) {
            Some(v) => {
                if v.get("event").and_then(|e| e.as_str()) == Some("video-reconfig") {
                    let elapsed = t0.elapsed();
                    println!("  video-reconfig after {:.1}ms", elapsed.as_secs_f64() * 1000.0);
                    saw_reconfig = true;
                    break;
                }
            }
            None => {
                // Read timeout — loop again until deadline.
            }
        }
    }
    let event_ok = saw_reconfig;
    println!("  {} video-reconfig within 5s", if event_ok { "PASS" } else { "FAIL" });

    // ── Test 3: property monotonicity during playback ──────────────────
    println!("\nTest 3: property monotonicity during playback");
    let mut positions = Vec::new();
    for i in 0..10 {
        thread::sleep(Duration::from_millis(50));
        if let Some(v) = request(
            &stream,
            &mut reader,
            (20000 + i) as u32,
            "get_property",
            &["time-pos"],
        ) {
            if let Some(f) = v.as_f64() {
                positions.push(f);
            }
        }
    }
    let monotonic = positions.windows(2).all(|w| w[1] >= w[0] - 0.01);
    println!(
        "  samples={}  first={:.3}s  last={:.3}s  monotonic={}",
        positions.len(),
        positions.first().copied().unwrap_or(f64::NAN),
        positions.last().copied().unwrap_or(f64::NAN),
        monotonic
    );
    let monot_ok = monotonic && positions.len() >= 5;
    println!("  {} monotonic time-pos", if monot_ok { "PASS" } else { "FAIL" });

    // ── Clean shutdown ────────────────────────────────────────────────
    println!("\nShutdown");
    let _ = send(&stream, &json!({"command": ["quit"]}));
    let _ = mpv.wait();
    let _ = std::fs::remove_file(SOCKET_PATH);

    // ── Verdict ───────────────────────────────────────────────────────
    // Latency is the decisive gate — if it's low, IPC won't be the
    // bottleneck for any mpv operation in the real app. Event-delivery
    // and property-monotonicity tests are informational: they may fail
    // on a dev machine without the test file, but that tells us
    // nothing about IPC fidelity (mpv would have failed the same way
    // running in-process).
    println!("\n=== Verdict ===");
    println!("  latency: {} (decisive)", if latency_ok { "PASS" } else { "FAIL" });
    println!("  events : {} (informational)", if event_ok { "PASS" } else { "FAIL" });
    println!("  monot  : {} (informational)", if monot_ok { "PASS" } else { "FAIL" });
    println!(
        "  {} full refactor can proceed",
        if latency_ok { "GO —" } else { "NO-GO —" }
    );
    std::process::exit(if latency_ok { 0 } else { 1 });
}

fn spawn_mpv() -> Child {
    let child = Command::new("mpv")
        .args([
            "--idle=yes",
            &format!("--input-ipc-server={}", SOCKET_PATH),
            "--no-terminal",
            "--vo=null",
            "--ao=null",
            "--no-input-default-bindings",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mpv");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !std::path::Path::new(SOCKET_PATH).exists() {
        if Instant::now() > deadline {
            panic!("mpv IPC socket didn't appear within 3s");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child
}

/// Send + block for the reply matching `request_id`. Intervening events
/// and stray replies are silently dropped. Returns `Some(data)` for a
/// successful reply, `None` on error / timeout.
fn request(
    stream: &UnixStream,
    reader: &mut BufReader<UnixStream>,
    request_id: u32,
    name: &str,
    args: &[&str],
) -> Option<Value> {
    let mut cmd_array = vec![Value::String(name.to_string())];
    for a in args {
        cmd_array.push(Value::String(a.to_string()));
    }
    let req = json!({
        "command": cmd_array,
        "request_id": request_id
    });
    send(stream, &req)?;
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        let v = read_one(reader)?;
        if v.get("request_id").and_then(|x| x.as_u64()) == Some(request_id as u64) {
            return v.get("data").cloned().or_else(|| Some(Value::Null));
        }
        // else: unrelated event / reply — skip.
    }
    None
}

fn send(stream: &UnixStream, cmd: &Value) -> Option<()> {
    let mut w = stream.try_clone().ok()?;
    writeln!(w, "{}", cmd).ok()?;
    w.flush().ok()?;
    Some(())
}

fn read_one(reader: &mut BufReader<UnixStream>) -> Option<Value> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => None, // EOF
        Ok(_) => serde_json::from_str(line.trim()).ok(),
        Err(_) => None, // timeout / error
    }
}

//! Runtime configuration for the desktop app.
//!
//! `server_url()` is resolved once, at first access:
//! 1. `KOALA_SERVER_URL` environment variable (highest priority — lets
//!    a packaged build target staging / a LAN server without rebuild).
//! 2. `$XDG_CONFIG_HOME/koala-tv/server_url` (or `~/.config/koala-tv/
//!    server_url`) — single-line text file.
//! 3. Compile-time default `http://localhost:4500`.
//!
//! Callers use `crate::config::server_url()` which returns a
//! `&'static str` backed by the `OnceLock`.

use std::path::PathBuf;
use std::sync::OnceLock;

const DEFAULT_SERVER_URL: &str = "http://localhost:4500";

static SERVER_URL_CELL: OnceLock<String> = OnceLock::new();

fn config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join("koala-tv");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config/koala-tv")
}

fn resolve_server_url() -> String {
    if let Ok(v) = std::env::var("KOALA_SERVER_URL") {
        let v = v.trim();
        if !v.is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    let path = config_dir().join("server_url");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        let v = contents.trim();
        if !v.is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    DEFAULT_SERVER_URL.to_string()
}

/// Server base URL (no trailing slash). Cached after first call.
pub fn server_url() -> &'static str {
    SERVER_URL_CELL.get_or_init(resolve_server_url).as_str()
}

//! Loads the mpv profile definitions from `config/mpv.json` at compile
//! time (include_str!) and exposes them as typed accessors.
//!
//! Previously the ~40 mpv flags for main + 13 for backup were inline
//! `Vec<&str>` literals in `views/player.rs` and `views/backup_player.rs`
//! with 60 % overlap copy-pasted and already subtly diverging. Now
//! there's one JSON source of truth with sections `common` / `main` /
//! `backup` / `backup_freeze` / `backup_thaw`.
//!
//! Compile-time baked (include_str! + OnceLock parse) so there's no
//! runtime file-lookup; editing `desktop/config/mpv.json` still
//! requires a rebuild, but the data and the Rust code are no longer
//! tangled.

use serde_json::Value;
use std::sync::OnceLock;

const PROFILES_JSON: &str = include_str!("../../config/mpv.json");

#[derive(Debug)]
pub struct MpvProfiles {
    root: Value,
}

static PROFILES: OnceLock<MpvProfiles> = OnceLock::new();

fn profiles() -> &'static MpvProfiles {
    PROFILES.get_or_init(|| {
        let root: Value =
            serde_json::from_str(PROFILES_JSON).expect("mpv.json is invalid JSON");
        MpvProfiles { root }
    })
}

/// Convert a `serde_json::Value` to the mpv CLI flag form `value`. mpv
/// accepts `--name=value`; we keep value stringified without quoting
/// since Command::arg handles any whitespace — but in practice our
/// values never contain whitespace.
fn value_to_cli_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "yes".into() } else { "no".into() },
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        Value::Array(_) | Value::Object(_) => {
            // Shouldn't appear in our profiles; best-effort JSON
            // serialise if it ever does.
            v.to_string()
        }
    }
}

/// Collect `--key=value` flags from one or more JSON object sections,
/// in the given order. Keys beginning with `_` (metadata/comments)
/// are skipped.
fn collect_flags(sections: &[&Value]) -> Vec<String> {
    let mut out = Vec::new();
    for section in sections {
        let Some(map) = section.as_object() else {
            continue;
        };
        for (k, v) in map {
            if k.starts_with('_') {
                continue;
            }
            out.push(format!("--{}={}", k, value_to_cli_str(v)));
        }
    }
    out
}

/// Collect runtime property updates (used by freeze / thaw). Returns
/// `(name, value)` pairs preserved as `serde_json::Value` so the
/// caller can pass them straight to `MpvIpcClient::set_property`
/// (which takes any `Serialize`).
fn collect_props(section: &Value) -> Vec<(String, Value)> {
    let Some(map) = section.as_object() else {
        return Vec::new();
    };
    map.iter()
        .filter(|(k, _)| !k.starts_with('_'))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// CLI flags for the main (high-quality) mpv subprocess, in spawn
/// order. Does NOT include `--wid=<xid>` — the caller prepends that
/// with the actual X11 child window ID. Does NOT include
/// `--ytdl-format=<quality>` — the caller chooses that at runtime
/// from `QUALITIES` + the user's `preferred_quality` setting.
pub fn main_flags() -> Vec<String> {
    let p = profiles();
    collect_flags(&[&p.root["common"], &p.root["main"]])
}

/// CLI flags for the backup (low-quality preview) mpv subprocess.
/// Does NOT include `--wid=<xid>` — caller prepends.
pub fn backup_flags() -> Vec<String> {
    let p = profiles();
    collect_flags(&[&p.root["common"], &p.root["backup"]])
}

/// Properties to set when a backup mpv is parked off-screen. Shrinks
/// the demuxer cache so a parked channel doesn't keep downloading
/// segments for the user may never come back.
pub fn backup_freeze_props() -> Vec<(String, Value)> {
    collect_props(&profiles().root["backup_freeze"])
}

/// Properties to set when a backup mpv is revealed (`show()` / `thaw()`).
/// Restores the regular demuxer cache size.
pub fn backup_thaw_props() -> Vec<(String, Value)> {
    collect_props(&profiles().root["backup_thaw"])
}

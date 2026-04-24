//! i18n lookup for the desktop app. Source of truth is
//! `shared/i18n/fr.json`; baked in at compile time via `include_str!`
//! and parsed once into a HashMap.
//!
//! Usage:
//!   use crate::i18n::t;
//!   t("chat.title")   // -> "Chat en direct"
//!
//! Missing keys return the key itself as a best-effort fallback —
//! makes gaps visible without crashing. Warnings go to `tracing::warn`
//! once per key (the `WARNED` set dedupes).

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

const BUNDLE_JSON: &str = include_str!("../../../shared/i18n/fr.json");

fn bundle() -> &'static serde_json::Value {
    static BUNDLE: OnceLock<serde_json::Value> = OnceLock::new();
    BUNDLE.get_or_init(|| {
        serde_json::from_str(BUNDLE_JSON).expect("shared/i18n/fr.json is invalid JSON")
    })
}

fn warned() -> &'static Mutex<HashSet<String>> {
    static WARNED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    WARNED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Look up a translation key. Returns an owned `String` so callers
/// can freely move it into GPUI element `.child(...)` / format!
/// arguments. If the key is missing, returns the key itself and
/// logs a warning once.
pub fn t(key: &str) -> String {
    let b = bundle();
    if let Some(v) = b.get(key).and_then(|v| v.as_str()) {
        return v.to_string();
    }
    if let Ok(mut w) = warned().lock() {
        if w.insert(key.to_string()) {
            tracing::warn!(key = %key, "i18n: missing key");
        }
    }
    key.to_string()
}

//! User settings — memory cache capacity + favourites list.
//!
//! Persisted to `~/.config/koala-tv/settings.json` for the
//! anonymous case. When a user is logged in, also synchronised with
//! the server via `GET/PUT /api/user/settings` so the same prefs
//! follow them across machines / fresh installs.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// How many channels can sit in the memory cache at once. `0`
    /// means the feature is disabled (no zap-back). Min "real" value
    /// is 2 = current channel + 1 previous (the 90 % "back to last
    /// channel" use case).
    #[serde(default = "default_memory_capacity")]
    pub memory_capacity: u8,
    /// Channel IDs the user has favourited. Rendered in the sidebar's
    /// "Favoris" section between Mémoire and TV.
    #[serde(default)]
    pub favorites: Vec<String>,
}

fn default_memory_capacity() -> u8 {
    2
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            memory_capacity: 2,
            favorites: Vec::new(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let mut path = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".config");
                p
            })
        })?;
    path.push("koala-tv");
    Some(path)
}

fn settings_file() -> Option<PathBuf> {
    let mut p = config_path()?;
    p.push("settings.json");
    Some(p)
}

/// Load settings from disk. Falls back to `Default` (memory_capacity=2,
/// no favourites) on any error — this is a soft preference, not data
/// the user can lose.
pub fn load() -> Settings {
    let Some(path) = settings_file() else {
        return Settings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// Persist settings to disk. Best-effort: errors are logged-and-ignored
/// (we'd rather lose a preference than crash the app).
pub fn save(settings: &Settings) {
    let Some(dir) = config_path() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(err = %e, "settings mkdir failed");
        return;
    }
    let Some(path) = settings_file() else {
        return;
    };
    let json = match serde_json::to_string_pretty(settings) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(err = %e, "settings serialise failed");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, json) {
        tracing::warn!(err = %e, "settings write failed");
    }
}

//! Persist `last_state_per_channel` across restarts.
//!
//! On a cold start, the startup HTTP prefetch takes ~100-500 ms to
//! populate the in-memory `last_state_per_channel`. Until it finishes,
//! the first click on any channel falls back to the server-driven path
//! (no optimistic instant-zap).
//!
//! By saving a snapshot of the map to disk on a debounced timer
//! during the session, a restart can warm the cache immediately from
//! disk. The HTTP prefetch still runs in parallel and overwrites each
//! entry with a fresh one when the response lands — disk values are
//! only a bootstrap so the rebase math (`seek_to += now - server_time`)
//! absorbs the staleness.

use crate::models::tv_state::TvState;
use std::collections::HashMap;
use std::path::PathBuf;

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".local");
                p.push("share");
                p
            })
        })?;
    let mut p = base;
    p.push("KoalaTV");
    Some(p)
}

fn cache_file() -> Option<PathBuf> {
    let mut p = cache_path()?;
    p.push("state_cache.json");
    Some(p)
}

/// Load the persisted per-channel state cache. Returns an empty map on
/// any error — this is a bootstrap optimisation, not data the user can
/// lose. Also returns empty if the file is older than 24 h since states
/// that stale would require more correction than they save.
pub fn load() -> HashMap<String, TvState> {
    let Some(path) = cache_file() else {
        return HashMap::new();
    };
    // Soft age cap: 24 h. Reasoning: `server_time` + `seek_to` rebase
    // works for minutes-to-hours of staleness; past that the odds of
    // playlist reshuffles (daily 3am cron) make the cached states
    // meaningless and we'd just flash a wrong frame before the
    // HTTP prefetch corrects it.
    if let Ok(meta) = std::fs::metadata(&path) {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = std::time::SystemTime::now().duration_since(modified) {
                if age > std::time::Duration::from_secs(24 * 3600) {
                    return HashMap::new();
                }
            }
        }
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Persist the map. Best-effort: errors are logged-and-ignored. Called
/// from a debounced timer in AppView so we don't hit the disk on every
/// tv:sync (4 per minute per channel × 48 channels = 192 writes/min
/// without debouncing).
pub fn save(states: &HashMap<String, TvState>) {
    let Some(dir) = cache_path() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(err = %e, "state_cache mkdir failed");
        return;
    }
    let Some(path) = cache_file() else {
        return;
    };
    let json = match serde_json::to_string(states) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(err = %e, "state_cache serialise failed");
            return;
        }
    };
    // Atomic write via tmp + rename so a crash mid-write doesn't
    // leave a truncated JSON that fails to parse on next boot.
    let mut tmp = path.clone();
    tmp.set_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, json) {
        tracing::warn!(err = %e, "state_cache write failed");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        tracing::warn!(err = %e, "state_cache rename failed");
    }
}

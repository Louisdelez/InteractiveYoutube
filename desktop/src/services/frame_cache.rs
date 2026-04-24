//! Per-favorite channel thumbnail cache.
//!
//! At click time the desktop paints a static JPEG of the target
//! channel *before* mpv has decoded a single frame — the user sees an
//! immediate visual change instead of the previous channel's frozen
//! last frame. Once mpv (main or backup) has its first frame, the
//! snapshot is cleared and real video takes over. ~0-ms perceived zap
//! on every favorite.
//!
//! Scoped to `settings.favorites` on purpose : the memory cost scales
//! linearly with the cache size (~150 KB per decoded JPEG), so we
//! don't warm all 48 channels. Typical user has 5-10 favorites →
//! ~1 MB client RAM for the whole feature.
//!
//! Image source : YouTube's own `img.youtube.com/vi/<id>/maxresdefault
//! .jpg` (fallback `hqdefault.jpg` — always present). No server
//! involvement : client talks direct to img.youtube.com. Same host
//! the user's mpv already resolves, so no new privacy surface.

use gpui::{Image, ImageFormat};
use std::collections::HashMap;
use std::sync::Arc;

/// One cached entry. `video_id` is the YouTube ID the JPEG represents
/// — when a channel auto-advances mid-session, we compare against
/// this and re-fetch if it's changed.
#[derive(Clone)]
pub struct FrameEntry {
    pub video_id: String,
    pub image: Arc<Image>,
}

#[derive(Default)]
pub struct FrameCache {
    entries: HashMap<String, FrameEntry>,
}

impl FrameCache {
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    pub fn get(&self, channel_id: &str) -> Option<&FrameEntry> {
        self.entries.get(channel_id)
    }

    pub fn insert(&mut self, channel_id: String, video_id: String, image: Arc<Image>) {
        self.entries.insert(channel_id, FrameEntry { video_id, image });
    }

    /// True if we have no entry OR the cached one is for a different
    /// videoId than `new_video_id` (auto-advance happened).
    pub fn needs_refresh(&self, channel_id: &str, new_video_id: &str) -> bool {
        self.entries
            .get(channel_id)
            .map(|e| e.video_id != new_video_id)
            .unwrap_or(true)
    }

    /// Drop any cached entry whose channel is no longer a favorite.
    /// Called after favorites are edited to free the RAM.
    pub fn evict_non_favorites(&mut self, favorites: &[String]) {
        self.entries.retain(|id, _| favorites.iter().any(|f| f == id));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Guess the format. YouTube serves JPEG for all `vi/<id>/*.jpg`
/// endpoints, but we sniff to be safe (and to handle future WebP
/// rollouts gracefully).
pub fn guess_format(bytes: &[u8]) -> ImageFormat {
    if bytes.len() >= 4 && &bytes[0..4] == b"\x89PNG" {
        ImageFormat::Png
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}

/// Blocking fetch (meant for a std::thread::spawn). Tries
/// `maxresdefault` first, falls back to `hqdefault` which YouTube
/// guarantees exists for every video.
pub fn fetch_thumbnail_bytes(video_id: &str) -> Option<Vec<u8>> {
    let hi = format!("https://img.youtube.com/vi/{}/maxresdefault.jpg", video_id);
    if let Ok(b) = crate::services::api::fetch_bytes(&hi) {
        if !b.is_empty() {
            return Some(b);
        }
    }
    let lo = format!("https://img.youtube.com/vi/{}/hqdefault.jpg", video_id);
    crate::services::api::fetch_bytes(&lo).ok()
}

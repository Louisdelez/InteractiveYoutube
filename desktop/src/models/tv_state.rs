use serde::{Deserialize, Serialize};

/// Authoritative TV state from the server. Contains extra metadata fields
/// (duration, indexes, etc.) that aren't all consumed by the desktop client.
/// `Serialize` is implemented so the `services::state_cache` module can
/// persist the per-channel cache to disk for instant-zap across restarts.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TvState {
    pub video_id: String,
    pub title: String,
    pub video_index: i32,
    pub seek_to: f64,
    pub duration: f64,
    pub embeddable: Option<bool>,
    pub server_time: u64,
    pub total_videos: usize,
    pub channel_id: String,
    pub is_priority: Option<bool>,
    /// ID of the next video in the channel's rotation (server pre-knows the
    /// playlist), used for gap-free prefetching via mpv's playlist + prefetch.
    pub next_video_id: Option<String>,
    pub next_title: Option<String>,
    pub next_duration: Option<f64>,
    pub published_at: Option<String>,
}

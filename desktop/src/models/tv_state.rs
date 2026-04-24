use serde::{Deserialize, Serialize};

/// Max age for a pre-resolved googlevideo URL before we stop trusting
/// it client-side. YouTube tokens typically live ~6 h; 5 h keeps a
/// safety margin. Overridable via `KOALA_RESOLVED_URL_MAX_AGE_SECS`
/// for ops tuning.
fn resolved_url_max_age_secs() -> u64 {
    std::env::var("KOALA_RESOLVED_URL_MAX_AGE_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5 * 3600)
}

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
    /// Pre-resolved googlevideo.com streaming URL (HQ progressive /
    /// HLS manifest), injected by the server's url-resolver worker.
    /// When present AND fresh (`resolved_at` < ~5 h old), the desktop
    /// passes it straight to mpv.loadfile with ytdl=no — bypasses the
    /// yt-dlp step entirely, shaving ~200-800 ms off the cold-zap
    /// first-frame latency. Absent → client falls back to
    /// `https://youtube.com/watch?v={video_id}` + ytdl_hook.
    pub resolved_url: Option<String>,
    /// Same idea, low-quality variant for the backup mpv.
    pub resolved_url_lq: Option<String>,
    /// UNIX seconds at which the URLs were resolved. Used client-side
    /// to trust the URL only within its safe window (YouTube tokens
    /// typically last ~6 h; we treat > 5 h as stale and fall back).
    pub resolved_at: Option<u64>,
}

impl TvState {
    /// True if `resolved_at` is present AND no older than the safety
    /// window. Callers must also check that the URL itself is non-empty.
    pub fn resolved_url_is_fresh(&self) -> bool {
        let Some(ts) = self.resolved_at else { return false };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now.saturating_sub(ts) < resolved_url_max_age_secs()
    }
}

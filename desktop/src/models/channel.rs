use serde::Deserialize;

/// Channel metadata received from the server's `GET /api/tv/channels`.
/// The server's `channels.json` is the single source of truth ; there
/// is NO client-side fallback list. A fresh desktop boot shows an
/// empty sidebar until the fetch completes (typically ~100-500 ms),
/// at which point the real list is installed.
#[derive(Clone, Debug, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub handle: String,
    pub avatar_url: String,
}

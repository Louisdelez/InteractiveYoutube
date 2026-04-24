use crate::config::server_url;
use crate::models::tv_state::TvState;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Shared reqwest client with cookie jar so login session persists across
/// requests (HTTP) and is sent automatically with every call.
static SHARED_CLIENT: OnceLock<Arc<Client>> = OnceLock::new();

fn shared_client() -> Arc<Client> {
    SHARED_CLIENT
        .get_or_init(|| {
            Arc::new(
                Client::builder()
                    .cookie_store(true)
                    .timeout(Duration::from_secs(8))
                    .build()
                    .expect("Failed to build reqwest client"),
            )
        })
        .clone()
}

fn client() -> Result<Arc<Client>, String> {
    Ok(shared_client())
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    #[allow(dead_code)]
    pub id: i64,
    pub username: String,
    #[allow(dead_code)]
    pub color: String,
}

#[derive(Debug, Deserialize)]
struct UserWrapper {
    user: User,
}

pub fn fetch_me() -> Result<User, String> {
    let url = format!("{}/api/auth/me", server_url());
    let resp = shared_client().get(&url).send().map_err(|e| e.to_string())?;
    if resp.status().as_u16() == 401 {
        return Err("not_authenticated".into());
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let w: UserWrapper = resp.json().map_err(|e| e.to_string())?;
    Ok(w.user)
}

#[derive(Serialize)]
struct LoginPayload<'a> {
    email: &'a str,
    password: &'a str,
}

pub fn login(email: &str, password: &str) -> Result<User, String> {
    let url = format!("{}/api/auth/login", server_url());
    let resp = shared_client()
        .post(&url)
        .json(&LoginPayload { email, password })
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let msg = resp
            .json::<serde_json::Value>()
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str().map(String::from)))
            .unwrap_or_else(|| format!("HTTP {}", status));
        return Err(msg);
    }
    let w: UserWrapper = resp.json().map_err(|e| e.to_string())?;
    Ok(w.user)
}

#[derive(Serialize)]
struct RegisterPayload<'a> {
    username: &'a str,
    email: &'a str,
    password: &'a str,
}

pub fn register(username: &str, email: &str, password: &str) -> Result<User, String> {
    let url = format!("{}/api/auth/register", server_url());
    let resp = shared_client()
        .post(&url)
        .json(&RegisterPayload { username, email, password })
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let msg = resp
            .json::<serde_json::Value>()
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str().map(String::from)))
            .unwrap_or_else(|| format!("HTTP {}", status));
        return Err(msg);
    }
    let w: UserWrapper = resp.json().map_err(|e| e.to_string())?;
    Ok(w.user)
}

/// Fetch the logged-in user's saved Settings from the server. Returns
/// `Ok(None)` if not authenticated.
pub fn fetch_user_settings() -> Result<Option<crate::services::settings::Settings>, String> {
    let url = format!("{}/api/user/settings", server_url());
    let resp = shared_client().get(&url).send().map_err(|e| e.to_string())?;
    if resp.status().as_u16() == 401 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    #[derive(serde::Deserialize)]
    struct Wrap {
        settings: serde_json::Value,
    }
    let w: Wrap = resp.json().map_err(|e| e.to_string())?;
    let settings: crate::services::settings::Settings =
        serde_json::from_value(w.settings).unwrap_or_default();
    Ok(Some(settings))
}

/// Push the user's Settings to the server. No-op for anonymous users
/// (server replies 401, which we swallow).
pub fn put_user_settings(settings: &crate::services::settings::Settings) -> Result<(), String> {
    let url = format!("{}/api/user/settings", server_url());
    let body = serde_json::json!({ "settings": settings });
    let resp = shared_client()
        .put(&url)
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?;
    if resp.status().as_u16() == 401 {
        return Ok(()); // not logged in — local-only is fine
    }
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}

pub fn logout() -> Result<(), String> {
    let url = format!("{}/api/auth/logout", server_url());
    let resp = shared_client().post(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn fetch_tv_state(channel_id: &str) -> Result<TvState, String> {
    let url = format!("{}/api/tv/state?channel={}", server_url(), channel_id);
    let resp = client()?.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), url));
    }
    resp.json::<TvState>().map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerChannel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub avatar: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistVideo {
    #[serde(rename = "videoId")]
    pub video_id: String,
    pub title: String,
    pub duration: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistInfo {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    #[serde(rename = "tvStartedAt")]
    pub tv_started_at: f64,
    #[serde(rename = "totalDuration")]
    pub total_duration: f64,
    pub videos: Vec<PlaylistVideo>,
}

pub fn fetch_playlist(channel_id: &str) -> Result<PlaylistInfo, String> {
    let url = format!("{}/api/tv/playlist?channel={}", server_url(), channel_id);
    let resp = client()?.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<PlaylistInfo>().map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Deserialize)]
pub struct GifResult {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub gif_url: String,
    pub preview_url: String,
    #[serde(default = "default_gif_dim")]
    pub width: u32,
    #[serde(default = "default_gif_dim")]
    pub height: u32,
}
fn default_gif_dim() -> u32 { 100 }

pub fn fetch_trending_gifs() -> Result<Vec<GifResult>, String> {
    let url = format!("{}/api/gifs/trending", server_url());
    let resp = client()?.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Vec<GifResult>>().map_err(|e| e.to_string())
}

pub fn search_gifs(query: &str) -> Result<Vec<GifResult>, String> {
    let url = format!("{}/api/gifs/search?q={}", server_url(), query);
    let resp = client()?.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Vec<GifResult>>().map_err(|e| e.to_string())
}

pub fn fetch_channels() -> Result<Vec<ServerChannel>, String> {
    let url = format!("{}/api/tv/channels", server_url());
    let resp = client()?.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Vec<ServerChannel>>().map_err(|e| e.to_string())
}

/// Download raw bytes of a URL (used for avatar images). On-disk cache
/// keyed by a content-free hash of the URL. Cache lives in
/// `$XDG_CACHE_HOME/koala-tv/avatars/` (fallback `~/.cache/…`) so avatars
/// don't need to be re-downloaded at every startup. Cache miss = HTTP
/// fetch; on success we best-effort write the bytes back to disk.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let cache_path = avatar_cache_path(url);
    if let Some(ref path) = cache_path {
        if let Ok(bytes) = std::fs::read(path) {
            if !bytes.is_empty() {
                return Ok(bytes);
            }
        }
    }
    let resp = client()?.get(url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), url));
    }
    let bytes: Vec<u8> = resp
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())?;
    if let Some(path) = cache_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &bytes);
    }
    Ok(bytes)
}

fn avatar_cache_path(url: &str) -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))?;
    // FNV-1a 64-bit — tiny, dependency-free, collision-irrelevant here.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    Some(base.join("koala-tv/avatars").join(format!("{:016x}", h)))
}

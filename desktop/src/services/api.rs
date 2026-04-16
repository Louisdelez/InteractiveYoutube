use crate::config::SERVER_URL;
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
    let url = format!("{}/api/auth/me", SERVER_URL);
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
    let url = format!("{}/api/auth/login", SERVER_URL);
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
    let url = format!("{}/api/auth/register", SERVER_URL);
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
    let url = format!("{}/api/user/settings", SERVER_URL);
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
    let url = format!("{}/api/user/settings", SERVER_URL);
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
    let url = format!("{}/api/auth/logout", SERVER_URL);
    let resp = shared_client().post(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn fetch_tv_state(channel_id: &str) -> Result<TvState, String> {
    let url = format!("{}/api/tv/state?channel={}", SERVER_URL, channel_id);
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
    let url = format!("{}/api/tv/playlist?channel={}", SERVER_URL, channel_id);
    let resp = client()?.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<PlaylistInfo>().map_err(|e| e.to_string())
}

pub fn fetch_channels() -> Result<Vec<ServerChannel>, String> {
    let url = format!("{}/api/tv/channels", SERVER_URL);
    let resp = client()?.get(&url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<Vec<ServerChannel>>().map_err(|e| e.to_string())
}

/// Download raw bytes of a URL (used for avatar images).
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = client()?.get(url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), url));
    }
    resp.bytes()
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())
}

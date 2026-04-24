//! Free helper functions extracted from `app.rs`. Latency indicators,
//! settings sync (HTTP → local), logo cache, image-format detection.
//! Moved here so the main `app.rs` stays focused on `AppView` setup +
//! render instead of mixing utility fns at the bottom.

use super::{api, settings, AppView, Settings};
use gpui::*;
use std::time::Duration;

/// Color for the WiFi-bars indicator + the ms label, based on round-trip
/// latency. None (= offline) is rendered red.
pub(super) fn latency_color(ms: Option<u32>) -> u32 {
    match ms {
        None => 0xef4444,           // red — offline
        Some(v) if v <= 80 => 0x10b981,   // green — excellent
        Some(v) if v <= 200 => 0xeab308,  // yellow — okay
        _ => 0xef4444,                    // red — bad
    }
}

/// 4-bar WiFi-style signal indicator. Bars fill in based on latency:
///   <= 60ms → 4 bars, <= 150ms → 3, <= 300ms → 2, <= 800ms → 1, else 0.
pub(super) fn signal_bars(latency: Option<u32>) -> impl IntoElement {
    let active_count: usize = match latency {
        None => 0,
        Some(v) if v <= 60 => 4,
        Some(v) if v <= 150 => 3,
        Some(v) if v <= 300 => 2,
        Some(v) if v <= 800 => 1,
        _ => 0,
    };
    let active_color = latency_color(latency);
    let inactive_color: u32 = 0x3a3a3f;
    let heights = [6.0_f32, 9.0, 12.0, 15.0];
    div()
        .flex()
        .items_end()
        .gap(px(2.0))
        .h(px(15.0))
        .children((0..4).map(|i| {
            let color = if i < active_count { active_color } else { inactive_color };
            div()
                .w(px(3.0))
                .h(px(heights[i]))
                .rounded(px(1.0))
                .bg(rgb(color))
        }).collect::<Vec<_>>())
}

/// Detect image format from first few bytes (magic numbers).
/// Pull the logged-in user's saved settings from the server (HTTP)
/// in a background thread; on success, apply them locally + persist
/// + push to player + sidebar. Server settings WIN over local on
/// authentication — that's the whole point of the sync.
pub(super) fn spawn_pull_user_settings(cx: &mut Context<AppView>) {
    let entity = cx.entity().downgrade();
    cx.spawn(async move |_, cx| {
        let (tx, rx) = std::sync::mpsc::channel::<Option<Settings>>();
        std::thread::spawn(move || {
            let s = api::fetch_user_settings().ok().flatten();
            let _ = tx.send(s);
        });
        loop {
            if let Ok(maybe) = rx.try_recv() {
                if let Some(s) = maybe {
                    if let Some(e) = entity.upgrade() {
                        let _ = cx.update(|cx| {
                            e.update(cx, |this: &mut AppView, cx| {
                                this.settings = s.clone();
                                settings::save(&this.settings);
                                #[cfg(target_os = "linux")]
                                this.player.update(cx, |p, _| {
                                    p.set_memory_capacity(s.memory_capacity);
                                });
                                if s.preferred_quality != 0 {
                                    let q = s.preferred_quality as usize;
                                    this.player.update(cx, |p, cx| p.set_quality(q, cx));
                                }
                                let favs = s.favorites.clone();
                                this.sidebar.update(cx, |s, cx| {
                                    s.set_favorites(favs);
                                    cx.notify();
                                });
                                cx.notify();
                            });
                        });
                    }
                }
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
        }
    })
    .detach();
}

/// Best-effort push of the current settings to the server in a
/// background thread. Anonymous users hit a 401 which is silently
/// ignored — local persistence still happens.
pub(super) fn spawn_push_user_settings(settings: Settings) {
    std::thread::spawn(move || {
        let _ = api::put_user_settings(&settings);
    });
}

/// App logo (koala) — embedded at compile time, decoded once.
const LOGO_PNG: &[u8] = include_bytes!("../../assets/koala-tv.png");
pub(super) fn koala_logo() -> std::sync::Arc<Image> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::sync::Arc<Image>> = OnceLock::new();
    CACHE
        .get_or_init(|| std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, LOGO_PNG.to_vec())))
        .clone()
}

pub(super) fn detect_image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.len() >= 8 && &bytes[0..8] == b"\x89PNG\r\n\x1a\n" {
        ImageFormat::Png
    } else if bytes.len() >= 4 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        ImageFormat::Jpeg
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
        ImageFormat::Gif
    } else {
        ImageFormat::Jpeg
    }
}

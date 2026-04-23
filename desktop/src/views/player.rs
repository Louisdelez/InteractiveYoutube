use gpui::*;
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use libmpv2::events::Event;
use libmpv2::Mpv;
use std::sync::{Arc, Mutex};

use crate::views::icons::{IconCache, IconName};

#[cfg(target_os = "linux")]
use crate::views::backup_player::BackupPlayer;
#[cfg(target_os = "linux")]
use crate::views::channel_badge::ChannelBadge;
#[cfg(target_os = "linux")]
use crate::views::loading_overlay::LoadingOverlay;
#[cfg(target_os = "linux")]
use crate::views::memory_cache::{MemorizedChannel, MemoryCache};
#[cfg(target_os = "linux")]
use crate::views::popup_menu::{MenuEvent, MenuKind, PopupMenu};

// ─── X11 error handling ──────────────────────────────────────────────
// Without a custom handler, Xlib's default prints the error and calls
// `exit(1)` — which is exactly the SIGSEGV-looking crash we saw on
// shutdown (BadWindow on X_DestroyWindow at serial ~112). At teardown
// time the parent GPUI window may already be gone, so the X server has
// auto-destroyed our child/sibling windows before our Drop impls run
// their own XDestroyWindow. That's a benign race, not a real bug —
// swallow it instead of aborting the process.
#[cfg(target_os = "linux")]
static X11_ERROR_HANDLER_INSTALLED: std::sync::Once = std::sync::Once::new();
#[cfg(target_os = "linux")]
pub(crate) static X11_SHUTTING_DOWN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "linux")]
unsafe extern "C" fn x11_error_handler(
    _display: *mut x11_dl::xlib::Display,
    event: *mut x11_dl::xlib::XErrorEvent,
) -> std::os::raw::c_int {
    // During shutdown, swallow everything silently.
    if X11_SHUTTING_DOWN.load(std::sync::atomic::Ordering::Acquire) {
        return 0;
    }
    let ev = &*event;
    // Runtime errors: log once and continue (never exit). BadWindow on
    // a stale resource is recoverable — the default handler aborting
    // the whole process is worse than a warning.
    tracing::warn!(
        code = ev.error_code,
        request = ev.request_code,
        resource = format!("0x{:x}", ev.resourceid),
        "X11 error"
    );
    0
}

#[cfg(target_os = "linux")]
fn install_x11_error_handler(xlib: &x11_dl::xlib::Xlib) {
    X11_ERROR_HANDLER_INSTALLED.call_once(|| unsafe {
        (xlib.XSetErrorHandler)(Some(x11_error_handler));
    });
}

// Layout constants (must match app.rs)
const SIDEBAR_W: f32 = 56.0;
const CHAT_W: f32 = 340.0;
const TOPBAR_H: f32 = 36.0;
const CONTROL_BAR_H: f32 = 48.0;
const INFOBAR_H: f32 = 36.0;


// Modern dark theme (spec matches YouTube/Twitch 2024+)
const BAR_BG: u32 = 0x0f0f11;
const BAR_BORDER: u32 = 0x1f1f23;
const BTN_HOVER: u32 = 0x26262b;
const BTN_ACTIVE: u32 = 0x33333a;
const TEXT_PRIMARY: u32 = 0xe8e8ea;
const TEXT_MUTED: u32 = 0xa8a8ad;
const ACCENT: u32 = 0x9b59b6;
const ICON_PX: u32 = 20;

/// Extract the YouTube video ID from a URL like "https://www.youtube.com/watch?v=XXXX".
/// Returns None for channel handles or non-watch URLs.
fn extract_video_id(url: &str) -> Option<String> {
    let after_v = url.split("watch?v=").nth(1)?;
    let id: String = after_v.chars().take_while(|c| *c != '&' && *c != '#').collect();
    if id.len() == 11 { Some(id) } else { None }
}

/// Open a URL in the user's default browser.
fn open_in_browser(url: &str) {
    // Spawn xdg-open in a detached thread that waits — without
    // wait()ing the child becomes a zombie that lingers in the
    // process table.
    let url = url.to_string();
    std::thread::spawn(move || {
        if let Ok(mut child) = std::process::Command::new("xdg-open").arg(&url).spawn() {
            let _ = child.wait();
        }
    });
}

/// Format a publishedAt ISO string (e.g. "2019-02-12T15:00:00Z") into
/// a French tooltip like "Mardi 12 février 2019 — il y a 6 ans".
fn format_published_tooltip(iso: &str) -> Option<String> {
    let date_part = iso.get(..10)?;
    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() != 3 { return None; }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;

    let months_fr = [
        "", "janvier", "février", "mars", "avril", "mai", "juin",
        "juillet", "août", "septembre", "octobre", "novembre", "décembre",
    ];
    let days_fr = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];

    // Zeller-like day of week (Tomohiko Sakamoto)
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let dow = ((y + y / 4 - y / 100 + y / 400 + t[month as usize - 1] + day as i32) % 7) as usize;
    // dow: 0=Sun..6=Sat → convert to 0=Mon..6=Sun
    let dow_mon = if dow == 0 { 6 } else { dow - 1 };
    let day_name = days_fr.get(dow_mon).unwrap_or(&"");
    let month_name = months_fr.get(month as usize).unwrap_or(&"");

    // Time elapsed
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let now_year = 1970 + (now.as_secs() / 31_557_600) as i32; // approx
    let diff_years = now_year - year;
    let ago = if diff_years >= 2 {
        format!("il y a {} ans", diff_years)
    } else if diff_years == 1 {
        "il y a 1 an".to_string()
    } else {
        let now_month = ((now.as_secs() % 31_557_600) / 2_629_800) as u32 + 1;
        let diff_months = (now_year - year) as u32 * 12 + now_month.saturating_sub(month);
        if diff_months > 1 {
            format!("il y a {} mois", diff_months)
        } else if diff_months == 1 {
            "il y a 1 mois".to_string()
        } else {
            "récente".to_string()
        }
    };

    Some(format!(
        "{} {} {} {} — {}",
        day_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
            + &day_name[1..],
        day,
        month_name,
        year,
        ago,
    ))
}

/// Append a debug line to /tmp/iyt-quality.log (used to diagnose the
/// dual-quality fallback). Best-effort, never panics.
fn log_quality(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/iyt-quality.log")
    {
        let _ = writeln!(
            f,
            "[{:?}] {}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            msg
        );
    }
}

/// Locate bundled mpv user-shaders next to the executable.
/// Returns a colon-separated path string for `glsl-shaders` or None if no
/// shaders are present.
fn bundled_shader_paths() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    // Look in <exe_dir>/shaders/ first (release layout), then in
    // <project_root>/assets/shaders/ (dev layout).
    let candidates = [
        exe_dir.join("shaders"),
        exe_dir.join("../../assets/shaders"),
        exe_dir.join("../../../assets/shaders"),
    ];
    for dir in candidates {
        if !dir.is_dir() {
            continue;
        }
        let mut paths = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("glsl") {
                    if let Some(s) = p.to_str() {
                        paths.push(s.to_string());
                    }
                }
            }
        }
        if !paths.is_empty() {
            paths.sort();
            return Some(paths.join(":"));
        }
    }
    None
}

/// Five "default" subtitle languages shown in the captions popup.
/// User can click "Plus de langues" to see everything else.
const COMMON_SUB_LANGS: &[&str] = &["fr", "en", "de", "es", "it"];

fn lang_display_name(code: &str) -> &str {
    match code {
        "fr" => "Français",
        "en" => "English",
        "de" => "Deutsch",
        "es" => "Español",
        "it" => "Italiano",
        "pt" => "Português",
        "ru" => "Русский",
        "ja" => "日本語",
        "ko" => "한국어",
        "zh" => "中文",
        "ar" => "العربية",
        "nl" => "Nederlands",
        "pl" => "Polski",
        "tr" => "Türkçe",
        "sv" => "Svenska",
        "no" => "Norsk",
        "da" => "Dansk",
        "fi" => "Suomi",
        "el" => "Ελληνικά",
        "he" => "עברית",
        "hi" => "हिन्दी",
        "th" => "ไทย",
        "vi" => "Tiếng Việt",
        "uk" => "Українська",
        "cs" => "Čeština",
        "hu" => "Magyar",
        "ro" => "Română",
        "id" => "Indonesia",
        _ => code,
    }
}

// **No AV1**: YouTube increasingly serves AV1 but GPUs older than
// NVIDIA Ampere (RTX 30-series) + AMD RDNA 2 have no hardware AV1
// decode. Without hwdec, mpv falls back to `dav1d` software decode
// which eats 30-40 % CPU even on 360p. The `[vcodec!*=av01]` filter
// excludes AV1 codecs from yt-dlp's format selection; the fallback
// chain gracefully degrades if no non-AV1 variant exists at that
// height. Users with Ampere+ GPUs are still fine on H.264/VP9 —
// they just don't get the bandwidth savings of AV1, which is a
// reasonable trade.
const QUALITIES: &[(&str, &str)] = &[
    (
        "Auto",
        "bestvideo[height<=1080][vcodec!*=av01]+bestaudio/best[height<=1080][vcodec!*=av01]/best[vcodec!*=av01]/best",
    ),
    (
        "1080p",
        "bestvideo[height<=1080][vcodec!*=av01]+bestaudio/best[height<=1080][vcodec!*=av01]",
    ),
    (
        "720p",
        "bestvideo[height<=720][vcodec!*=av01]+bestaudio/best[height<=720][vcodec!*=av01]",
    ),
    (
        "480p",
        "bestvideo[height<=480][vcodec!*=av01]+bestaudio/best[height<=480][vcodec!*=av01]",
    ),
    (
        "360p",
        "bestvideo[height<=360][vcodec!*=av01]+bestaudio/best[height<=360][vcodec!*=av01]",
    ),
];

pub struct PlayerView {
    pub title: String,
    pub published_at: Option<String>,
    mpv: Arc<Mutex<Mpv>>,
    current_url: String,
    /// Bare YouTube video ID for "open in browser" link. None when the current
    /// URL is a channel handle (no specific video).
    current_video_id: Option<String>,
    /// Video ID we've already appended to mpv's playlist for prefetch.
    /// Avoids re-appending the same next entry on every tv:sync tick.
    queued_next_id: Option<String>,
    /// What mpv's `path` property reported on the previous poll, used to
    /// detect auto-advance to the next playlist entry.
    last_observed_video_id: Option<String>,
    /// Secondary mpv instance running the lowest-quality stream of the
    /// same video, ready to be raised on cache stall for an instant swap.
    #[cfg(target_os = "linux")]
    backup: Option<BackupPlayer>,
    /// Cache of last-N channels visited. When the user switches away
    /// from a channel, its backup mpv (low-quality, already decoding)
    /// is `freeze()`d and parked here; clicking it again `thaw()`s
    /// for an instant zap.
    #[cfg(target_os = "linux")]
    memory_cache: MemoryCache,
    /// X11 handle of the GPUI parent window — needed to spawn fresh
    /// `BackupPlayer` instances at runtime when a never-cached channel
    /// is visited.
    #[cfg(target_os = "linux")]
    parent_wid: std::ffi::c_ulong,
    /// channel_id currently active. Used to detect channel changes
    /// (different from video_id which can change within a channel via
    /// auto-advance).
    current_channel_id: Option<String>,
    /// Black X11 sibling window (with a "Chargement…" label) raised
    /// above mpv during channel switches. PURELY visual — never touches
    /// any mpv state. Hidden by default; the switch state machine
    /// shows it after a 400 ms grace period and hides it the moment
    /// backup mpv signals a frame is on screen.
    #[cfg(target_os = "linux")]
    loading_overlay: Option<LoadingOverlay>,
    /// Channel name + avatar overlaid in the top-left of the video
    /// area. Implemented as another X11 sibling above mpv.
    #[cfg(target_os = "linux")]
    channel_badge: Option<ChannelBadge>,
    /// Wall-clock instant when mpv first reported `paused-for-cache=true`
    /// for the current video. Used to debounce the fallback swap.
    cache_stall_since: Option<std::time::Instant>,
    /// True when we've already swapped to the backup low-quality track for
    /// the current video.
    using_backup: bool,
    /// Wall-clock instant of the last swap to backup. Used to retry the
    /// main stream periodically when the network recovers.
    backup_since: Option<std::time::Instant>,
    quality_idx: usize,
    captions_on: bool,
    volume: i64,
    sub_label: String,
    audio_label: String,
    volume_state: Entity<SliderState>,
    icons: IconCache,
    captions_open: bool,
    audio_open: bool,
    quality_open: bool,
    /// When true, the captions popup lists every available language; otherwise
    /// it only shows the 5 common ones (fr/en/de/es/it).
    show_all_sub_langs: bool,
    #[cfg(target_os = "linux")]
    popup: Option<std::rc::Rc<std::cell::RefCell<PopupMenu>>>,
    /// Last known control-bar geometry in window coords — used as anchor for
    /// opening popup menus from GPUI click handlers.
    control_bar_y: i32,
    control_bar_right: i32,
    #[cfg(target_os = "linux")]
    child_window: std::ffi::c_ulong,
    #[cfg(target_os = "linux")]
    xlib: Arc<x11_dl::xlib::Xlib>,
    #[cfg(target_os = "linux")]
    display: *mut x11_dl::xlib::Display,
    last_area: Option<(i32, i32, u32, u32)>,
    /// Set when a modal is open — prevents apply_geometry from
    /// dragging the off-screen mpv child window back into view on
    /// the next render.
    video_hidden: bool,
    /// Whether the chat sidebar is currently visible. Drives the player
    /// width — when false, mpv extends over the 340 px chat area.
    chat_open: bool,
    /// Hard fallback deadline for the loading overlay (so the screen never
    /// stays black forever if mpv silently fails). The overlay also clears
    /// as soon as `loading_for_video` is recognised in mpv's `path`.
    loading_until: Option<std::time::Instant>,
    /// The earliest moment the spinner is allowed to appear on screen.
    /// We keep showing the previous video's last frame for ~500 ms after
    /// a switch — if backup mpv finishes buffering before this delay,
    /// the spinner *never* appears (industry-standard pattern).
    loading_show_after: Option<std::time::Instant>,
    // ── Channel-switch loading overlay state ──────────────────────────
    /// Wall-clock instant when a channel switch was requested. `None` =
    /// no switch in progress. The render loop uses this to decide whether
    /// to show the spinner (delayed-spinner pattern: 400 ms grace period
    /// where the previous frame stays visible; spinner only appears if
    /// backup mpv hasn't loaded by then).
    switch_arm_at: Option<std::time::Instant>,
    /// Wall-clock instant when the spinner overlay actually became
    /// visible (so we can enforce a 500 ms minimum visible duration —
    /// avoids a sub-second flash that feels like a glitch).
    switch_overlay_shown_at: Option<std::time::Instant>,
    /// Wall-clock instant when the backup mpv reported the new video is
    /// rendering (via `MPV_EVENT_PLAYBACK_RESTART`). `None` = not ready.
    switch_backup_ready_at: Option<std::time::Instant>,
    /// True once main mpv has fired `MPV_EVENT_PLAYBACK_RESTART` for the
    /// CURRENT video — i.e., main is actually rendering, not just
    /// "demuxer cache filled". Drives the swap-up to main: we wait for
    /// this signal instead of guessing from `demuxer-cache-time`.
    /// Reset to `false` whenever a new main loadfile is kicked.
    main_first_frame_ready: bool,
    /// When `Some(video_id)` the player keeps the loading overlay up until
    /// either main or backup mpv reports playing this video. Set on channel
    /// switch in `load_state`, cleared by the poll loop.
    loading_for_video: Option<String>,
    /// mpv's `path` property snapshot at the moment a switch was requested.
    /// We clear the loading overlay only once mpv reports a DIFFERENT path
    /// (proving the loadfile actually took effect — comparing by YouTube
    /// video id doesn't work because mpv's path is the resolved googlevideo
    /// URL after yt-dlp).
    loading_pre_path: Option<String>,
    /// Same idea for the backup mpv — its path differs from main's because
    /// it uses a different format selector, so we have to snapshot both.
    loading_pre_path_backup: Option<String>,
    /// True between a channel switch and the moment backup mpv is
    /// actually rendering the new content. While true, the poll loop
    /// keeps backup hidden (so the user keeps seeing the previous video
    /// instead of backup's stale last frame); when backup starts
    /// rendering, the poll loop reveals it (`b.show()`).
    pending_backup_reveal: bool,
    /// Main mpv's pending loadfile (URL, seek_to). We defer the main
    /// loadfile until backup is on screen — otherwise main switches to
    /// the new URL but stays frozen on the previous channel's last frame
    /// for 1-3 s, which is exactly what the user sees as "trop long".
    pending_main_load: Option<(String, f64)>,
    #[allow(dead_code)]
    _subs: Vec<Subscription>,
}

unsafe impl Send for PlayerView {}
unsafe impl Sync for PlayerView {}

/// Emitted when mpv silently moved to the prefetched next playlist entry.
/// AppView listens and asks the server for a fresh tv:state so any drift
/// gets corrected immediately (instead of waiting for the next tv:sync tick).
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct AutoAdvanced {
    pub new_video_id: String,
}

impl EventEmitter<AutoAdvanced> for PlayerView {}

/// Emitted whenever the memory cache contents change (channel pushed,
/// taken, or evicted). The payload is the channel-id list, most-recent
/// first, so the sidebar can render its "Mémoire" section.
#[derive(Clone, Debug)]
pub struct MemoryChanged(pub Vec<String>);

impl EventEmitter<MemoryChanged> for PlayerView {}

/// Emitted when the user clicks the star icon in the channel badge —
/// AppView toggles the favourite status for the current channel.
#[derive(Clone, Debug)]
pub struct FavoriteToggleFromBadge;

impl EventEmitter<FavoriteToggleFromBadge> for PlayerView {}

#[cfg(target_os = "linux")]
impl Drop for PlayerView {
    fn drop(&mut self) {
        // Mark teardown so the global X11 error handler swallows benign
        // BadWindow races (parent GPUI window may already be gone, so
        // the X server has auto-destroyed our descendants).
        X11_SHUTTING_DOWN.store(true, std::sync::atomic::Ordering::Release);

        // Rust drops struct fields AFTER this user Drop body. Our
        // fields (backup, loading_overlay, channel_badge, popup, and
        // every cached backup in memory_cache) each XDestroyWindow on
        // the shared `display` we're about to close. Drop them here
        // FIRST so their Drop impls run against a live display.
        self.backup.take();
        self.loading_overlay.take();
        self.channel_badge.take();
        self.popup.take();
        self.memory_cache.clear();

        unsafe {
            (self.xlib.XDestroyWindow)(self.display, self.child_window);
            (self.xlib.XFlush)(self.display);
            (self.xlib.XCloseDisplay)(self.display);
        }
    }
}

impl PlayerView {
    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let initial_video = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

        #[cfg(target_os = "linux")]
        let (mpv, child_window, xlib, display, popup, popup_rx, backup, loading_overlay, channel_badge, parent_wid_out) = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};

            let raw_wh = window.window_handle().expect("No window handle");
            let parent_wid: std::ffi::c_ulong = match raw_wh.as_raw() {
                RawWindowHandle::Xcb(h) => h.window.get() as std::ffi::c_ulong,
                RawWindowHandle::Xlib(h) => h.window,
                _ => panic!("Unsupported window handle on Linux"),
            };

            let xlib = Arc::new(x11_dl::xlib::Xlib::open().expect("Failed to open Xlib"));
            install_x11_error_handler(&xlib);
            let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
            if display.is_null() {
                panic!("Could not open X display");
            }

            let child_window = unsafe {
                // ARGB-aware opaque black (see backup_player.rs for
                // the same reasoning) — XBlackPixel is fully
                // transparent on 32-bit visuals, leaving holes through
                // the app to the desktop while mpv hasn't yet
                // rendered a frame after a channel switch.
                let mut attrs: x11_dl::xlib::XWindowAttributes = std::mem::zeroed();
                (xlib.XGetWindowAttributes)(display, parent_wid, &mut attrs);
                let opaque_black: std::ffi::c_ulong = if attrs.depth >= 32 {
                    0xFF000000
                } else {
                    (xlib.XBlackPixel)(display, (xlib.XDefaultScreen)(display))
                };
                let w = (xlib.XCreateSimpleWindow)(
                    display,
                    parent_wid,
                    SIDEBAR_W as i32,
                    TOPBAR_H as i32,
                    800,
                    600,
                    0,
                    opaque_black,
                    opaque_black,
                );
                (xlib.XSetWindowBackground)(display, w, opaque_black);
                (xlib.XMapWindow)(display, w);
                (xlib.XClearWindow)(display, w);
                (xlib.XFlush)(display);
                w
            };

            let mpv = Mpv::with_initializer(|init| {
                init.set_property("wid", child_window as i64)?;
                init.set_property("ytdl", "yes")?;
                init.set_property("ytdl-format", QUALITIES[0].1)?; // default = Auto
                init.set_property("osc", "no")?;
                init.set_property("input-default-bindings", "no")?;
                init.set_property("input-vo-keyboard", "no")?;
                init.set_property("cursor-autohide", "no")?;
                init.set_property("force-window", "yes")?;
                init.set_property("idle", "yes")?;
                init.set_property("keep-open", "no")?;
                init.set_property("hwdec", "auto-safe")?;
                init.set_property("volume", 100i64)?;
                // Gap-free transitions: when the next playlist entry is queued
                // via `loadfile <url> append-play`, mpv silently runs yt-dlp +
                // opens the demuxer + prebuffers, so EOF → next is instant.
                init.set_property("prefetch-playlist", "yes")?;
                init.set_property("cache", "yes")?;
                // Cache sizing trades memory/threads vs. blip-resistance.
                // libmpv's HLS/DASH demuxer keeps one in-flight segment
                // thread per ~few seconds of readahead; cache-secs=60 +
                // readahead=20 used to translate to ~80 demux threads
                // per mpv instance (main + backup = ~160 at idle). 30 s
                // of readahead still survives a 30 s network blip on a
                // single video and halves the thread footprint.
                init.set_property("cache-secs", 30i64)?;
                init.set_property("demuxer-readahead-secs", 15i64)?;
                init.set_property("demuxer-max-bytes", "200MiB")?;
                init.set_property("demuxer-max-back-bytes", "50MiB")?;
                init.set_property(
                    "stream-lavf-o",
                    "reconnect=1,reconnect_streamed=1,reconnect_delay_max=5",
                )?;
                // Don't pause to wait for cache to fill before playing —
                // start as soon as we have enough to decode one frame.
                // Cuts ~500 ms off perceived startup latency.
                init.set_property("cache-pause-initial", "no")?;
                init.set_property("cache-pause-wait", 1.0)?;
                // Keyframe-only seek on swap = no decoder flush + no
                // black flash. Slight position imprecision is masked by
                // our drift-aware logic in swap-up/down.
                init.set_property("hr-seek", "no")?;
                init.set_property("network-timeout", 10i64)?;
                // gpu-next supports the VRR/swap-chain hacks that shave
                // ~1 vsync of latency. Safe for VOD content.
                init.set_property("video-latency-hacks", "yes")?;

                // ── "Browser-like" rendering ──────────────────────────────
                // Goal: match Chrome's <video> output as closely as possible
                // for YouTube content (no extra sharpening, no tone mapping,
                // no gamut clipping, no frame interpolation). YouTube already
                // bakes its own softness into the VP9/AV1 re-encode — adding
                // mpv post-processing makes the image diverge from what users
                // expect. "Decode and display, add nothing."
                init.set_property("vo", "gpu-next")?;
                init.set_property("profile", "fast")?;

                // Bilinear is what the browser does on the GPU compositor.
                init.set_property("scale", "bilinear")?;
                init.set_property("dscale", "bilinear")?;
                init.set_property("cscale", "bilinear")?;
                init.set_property("sigmoid-upscaling", "no")?;
                init.set_property("correct-downscaling", "no")?;
                init.set_property("linear-downscaling", "no")?;
                init.set_property("deband", "no")?;
                init.set_property("dither-depth", "auto")?;

                // Color: let the source/display speak for themselves.
                // Chrome does no tone/gamut mapping on Linux without an ICC
                // profile — neither should we.
                init.set_property("target-colorspace-hint", "no")?;
                init.set_property("tone-mapping", "auto")?;
                init.set_property("gamut-mapping-mode", "auto")?;

                // Motion: native cadence, no interpolation (no soap-opera).
                init.set_property("video-sync", "audio")?;
                init.set_property("interpolation", "no")?;

                // Anime4K shader stays bundled but un-loaded by default.
                let _ = bundled_shader_paths;
                init.set_property("sub-visibility", false)?;
                init.set_property("sub-auto", "all")?;
                // YouTube exposes auto-translations to ~200 languages. Asking
                // for `all` triggers yt-dlp to push them all as subtitle tracks
                // into mpv (slow + noisy). We pass a SINGLE wildcard token (no
                // comma — mpv's ytdl-raw-options parser splits values on
                // commas). Then we further filter the list at display time
                // (see `list_sub_tracks`).
                init.set_property(
                    "ytdl-raw-options",
                    "sub-langs=all,write-auto-subs=,write-subs=",
                )?;
                // Point mpv's bundled ytdl_hook.lua at our auto-updated binary
                // (see services::ytdlp_updater). Falls back to $PATH resolution
                // if the file isn't there yet on first boot.
                let ytdl_path = crate::services::ytdlp_updater::binary_path();
                if ytdl_path.exists() {
                    init.set_property(
                        "script-opts",
                        format!("ytdl_hook-ytdl_path={}", ytdl_path.display())
                            .as_str(),
                    )?;
                }
                Ok(())
            })
            .expect("Failed to init mpv");

            // No `loadfile` at startup — server is the source of truth.
            // `force-window=yes` (set in init) gives us an empty mpv window
            // until the first tv:state arrives; no Rick Astley, no random
            // content.
            let _ = initial_video;

            // X11 popup menu (sibling of mpv, raised above the video).
            let (popup, popup_rx) = match PopupMenu::new(parent_wid) {
                Some((p, r)) => (Some(std::rc::Rc::new(std::cell::RefCell::new(p))), Some(r)),
                None => (None, None),
            };

            // Secondary low-quality mpv instance, also waiting for the
            // first tv:state — no preload.
            let backup = BackupPlayer::new(parent_wid, xlib.clone(), display);

            // Loading-screen overlay: a black X11 sibling above mpv,
            // shown during channel switches to hide the brief decode/
            // re-buffer artefacts. Never touches mpv.
            let loading_overlay =
                LoadingOverlay::new(parent_wid, xlib.clone(), display);

            // "Now playing" badge — also an X11 sibling above mpv,
            // positioned in the top-left of the video area.
            let channel_badge =
                ChannelBadge::new(parent_wid, xlib.clone(), display);

            (
                Arc::new(Mutex::new(mpv)),
                child_window,
                xlib,
                display,
                popup,
                popup_rx,
                backup,
                loading_overlay,
                channel_badge,
                parent_wid,
            )
        };

        #[cfg(not(target_os = "linux"))]
        let mpv = {
            let mpv = Mpv::with_initializer(|init| {
                init.set_property("ytdl", "yes")?;
                init.set_property("osc", "no")?;
                Ok(())
            })
            .expect("Failed to init mpv");
            // Same reason: don't auto-play anything. Wait for tv:state.
            let _ = initial_video;
            Arc::new(Mutex::new(mpv))
        };

        cx.new(|cx| {
            #[cfg(target_os = "linux")]
            {
                // Background poll: X11 popup events + mpv auto-advance detection.
                let pop_rx = popup_rx.map(|r| std::sync::Arc::new(std::sync::Mutex::new(r)));
                let this_entity = cx.entity().downgrade();
                cx.spawn(async move |_, cx| {
                    // Adaptive poll cadence. During "critical" phases
                    // (channel switching, cache stall detection, pending
                    // main-load, swap-up race) we need fine-grained 16 ms
                    // sampling — those windows are sub-second and a slow
                    // poll would miss the ready-signal edge. Idle, we
                    // drop to 60 ms (≈16 Hz) which is enough to drain
                    // mpv events + refresh overlays, and avoids burning
                    // ~5 ms CPU per tick on the UI thread (≈ 30% of a
                    // frame budget at 60 FPS).
                    let critical_interval = std::time::Duration::from_millis(16);
                    let idle_interval = std::time::Duration::from_millis(60);
                    let critical_flag: std::sync::Arc<std::sync::atomic::AtomicBool> =
                        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    loop {
                        let entity = this_entity.clone();
                        let pop_rx = pop_rx.clone();
                        let crit = critical_flag.clone();
                        cx.update(move |cx| {
                            if let Some(e) = entity.upgrade() {
                                e.update(cx, |p: &mut PlayerView, cx| {
                                    if let Some(pop) = p.popup.as_ref() {
                                        pop.borrow_mut().pump();
                                    }

                                    // Drain mpv main events — watch for
                                    // VideoReconfig (= first decoded
                                    // frame is ready for the VO) so
                                    // swap-up only fires when main can
                                    // actually paint.
                                    let mut just_became_ready = false;
                                    if let Ok(mut mpv) = p.mpv.lock() {
                                        loop {
                                            match mpv.wait_event(0.0) {
                                                Some(Ok(Event::VideoReconfig)) => {
                                                    if !p.main_first_frame_ready {
                                                        just_became_ready = true;
                                                    }
                                                    p.main_first_frame_ready = true;
                                                }
                                                Some(_) => continue,
                                                None => break,
                                            }
                                        }
                                    }
                                    // First-frame fade-in: when main mpv
                                    // produces its first decoded frame
                                    // AND backup is not currently the
                                    // visible source, ramp main's audio
                                    // from 0 → 100 over 200 ms. Avoids
                                    // the audible burst when video
                                    // suddenly appears.
                                    if just_became_ready && !p.using_backup {
                                        if let Ok(m) = p.mpv.lock() {
                                            let _ = m.set_property("mute", false);
                                        }
                                        fade_volume(p.mpv.clone(), 0, 100, 200, cx);
                                    }

                                    // If a channel switch is pending,
                                    // poll backup for first-frame
                                    // readiness. The moment it has a
                                    // decoded frame, raise it (so the
                                    // user sees the new channel) AND
                                    // kick the deferred main loadfile
                                    // (so main loads silently under the
                                    // backup and the existing swap-up
                                    // can later promote it to high-res).
                                    if p.pending_backup_reveal {
                                        let backup_ready = p
                                            .backup
                                            .as_mut()
                                            .map(|b| b.poll_first_frame_ready())
                                            .unwrap_or(true);
                                        if backup_ready {
                                            if let Some(b) = p.backup.as_mut() {
                                                b.show();
                                            }
                                            p.using_backup = true;
                                            p.cache_stall_since = None;
                                            p.backup_since = Some(std::time::Instant::now());
                                            p.pending_backup_reveal = false;
                                            cx.notify();
                                        }
                                    }

                                    // Detect mpv auto-advance to the prefetched
                                    // next playlist entry by watching `path`.
                                    let current_mpv_path = p
                                        .mpv
                                        .lock()
                                        .ok()
                                        .and_then(|m| m.get_property::<String>("path").ok())
                                        .unwrap_or_default();
                                    let observed = extract_video_id(&current_mpv_path);

                                    // Clear the loading overlay only once
                                    // mpv (main OR backup) has switched off
                                    // the previous channel's content. We
                                    // detect this by comparing mpv's `path`
                                    // to the snapshot taken at switch time:
                                    // a different *non-empty* path that's
                                    // also actively decoding (paused-for-
                                    // cache=false, time-pos > 0.5) means the
                                    // new video is on screen.
                                    if p.loading_for_video.is_some() {
                                        let pre_main = p.loading_pre_path.clone().unwrap_or_default();
                                        let pre_backup = p
                                            .loading_pre_path_backup
                                            .clone()
                                            .unwrap_or_default();

                                        let main_ok = !current_mpv_path.is_empty()
                                            && current_mpv_path != pre_main
                                            && p.mpv.lock().ok().map(|m| {
                                                // "Actively rendering" =
                                                // not idle, not buffering,
                                                // not seeking. time-pos is
                                                // unreliable: it jumps to
                                                // `start` instantly on
                                                // loadfile.
                                                let core_idle = m
                                                    .get_property::<bool>("core-idle")
                                                    .unwrap_or(true);
                                                let stalled = m
                                                    .get_property::<bool>("paused-for-cache")
                                                    .unwrap_or(false);
                                                let seeking = m
                                                    .get_property::<bool>("seeking")
                                                    .unwrap_or(false);
                                                !core_idle && !stalled && !seeking
                                            }).unwrap_or(false);

                                        let backup_ok = p.backup.as_ref().map(|b| {
                                            b.is_playing_different_from(&pre_backup)
                                        }).unwrap_or(false);

                                        if main_ok || backup_ok {
                                            p.loading_for_video = None;
                                            p.loading_until = None;
                                            p.loading_pre_path = None;
                                            p.loading_pre_path_backup = None;
                                            cx.notify();
                                        }
                                    }

                                    // (duplicate `pending_backup_reveal`
                                    // handler removed — the canonical
                                    // one above uses the cleaner
                                    // `poll_first_frame_ready` based on
                                    // mpv's VIDEO_RECONFIG event)
                                    if observed.is_some() && observed != p.last_observed_video_id {
                                        p.last_observed_video_id = observed.clone();
                                        if let Some(new_id) = observed {
                                            if p.current_video_id.as_deref() != Some(new_id.as_str()) {
                                                p.current_video_id = Some(new_id.clone());
                                                p.queued_next_id = None;
                                                // Re-arm backup mpv on the new video.
                                                p.cache_stall_since = None;
                                                p.using_backup = false;
                                                // The new video hasn't been
                                                // VIDEO_RECONFIG'd yet — clear
                                                // the flag so the swap-up timer
                                                // doesn't fire on a stale signal.
                                                p.main_first_frame_ready = false;
                                                p.attach_backup_quality(&new_id, cx);
                                                cx.emit(AutoAdvanced { new_video_id: new_id });
                                            }
                                        }
                                    }

                                    // Adaptive fallback using the parallel
                                    // BackupPlayer (a 2nd mpv instance running
                                    // a low-quality stream with its own X11
                                    // window). On stall: raise + unmute the
                                    // backup, mute the main. After 8 s: swap
                                    // back. The backup is ALREADY decoding so
                                    // the swap is sub-100 ms.
                                    if p.backup.is_some() {
                                        let stalled = p
                                            .mpv
                                            .lock()
                                            .ok()
                                            .and_then(|m| {
                                                m.get_property::<bool>("paused-for-cache").ok()
                                            })
                                            .unwrap_or(false);
                                        let now = std::time::Instant::now();

                                        if !p.using_backup {
                                            if stalled {
                                                let since = p
                                                    .cache_stall_since
                                                    .get_or_insert(now);
                                                if now.duration_since(*since)
                                                    >= std::time::Duration::from_secs(2)
                                                {
                                                    let main_pos = p
                                                        .mpv
                                                        .lock()
                                                        .ok()
                                                        .and_then(|m| {
                                                            m.get_property::<f64>("time-pos").ok()
                                                        })
                                                        .unwrap_or(0.0);
                                                    let backup_pos = p
                                                        .backup
                                                        .as_ref()
                                                        .and_then(|b| b.time_pos())
                                                        .unwrap_or(0.0);
                                                    let drift = (backup_pos - main_pos).abs();
                                                    if let Some(b) = p.backup.as_mut() {
                                                        // Drift-aware: only seek if
                                                        // backup is >500ms off main.
                                                        // Within 500ms, mpv just keeps
                                                        // playing — no decoder flush,
                                                        // no black flash from hr-seek.
                                                        if drift > 0.5 && main_pos > 0.5 {
                                                            b.seek(main_pos);
                                                        }
                                                        b.show();
                                                    }
                                                    // Audio crossfade 80ms — main
                                                    // fades to silent, backup fades
                                                    // up. Avoids the audible pop of
                                                    // hard mute toggle.
                                                    fade_volume(p.mpv.clone(), 100, 0, 80, cx);
                                                    if let Some(b) = p.backup.as_ref() {
                                                        fade_volume(b.mpv.clone(), 0, 100, 80, cx);
                                                    }
                                                    p.using_backup = true;
                                                    p.cache_stall_since = None;
                                                    p.backup_since = Some(now);
                                                    log_quality(&format!(
                                                        "SWAP DOWN t={:.1}s drift={:.0}ms",
                                                        main_pos, drift * 1000.0
                                                    ));
                                                }
                                            } else {
                                                p.cache_stall_since = None;
                                            }
                                        } else if let Some(since) = p.backup_since {
                                            // Only swap back once we've been on
                                            // backup for at least 3 s AND the
                                            // main mpv has actually finished
                                            // buffering (not paused-for-cache
                                            // anymore + has some demuxer cache).
                                            // This way a quality change won't
                                            // pop back to a still-loading main.
                                            let elapsed = now.duration_since(since);
                                            // Require main mpv to have
                                            // fired VIDEO_RECONFIG (= it
                                            // has actually decoded a
                                            // frame of the NEW video).
                                            // Otherwise swap-up would
                                            // unmap backup and reveal
                                            // main's stale previous
                                            // frame for 1-3 s ("frame
                                            // de l'ancien stream").
                                            let main_ready = !stalled
                                                && p.main_first_frame_ready;
                                            if elapsed
                                                >= std::time::Duration::from_secs(3)
                                                && main_ready
                                            {
                                                let backup_pos = p
                                                    .backup
                                                    .as_ref()
                                                    .and_then(|b| b.time_pos())
                                                    .unwrap_or(0.0);
                                                let main_pos_now = p
                                                    .mpv
                                                    .lock()
                                                    .ok()
                                                    .and_then(|m| m.get_property::<f64>("time-pos").ok())
                                                    .unwrap_or(0.0);
                                                let drift = (backup_pos - main_pos_now).abs();
                                                // Drift-aware seek: only realign
                                                // main to backup if they're more
                                                // than 300ms apart. Within that
                                                // window, the inevitable seek-
                                                // induced black flash is worse
                                                // than the imperceptible audio
                                                // drift.
                                                if let Ok(m) = p.mpv.lock() {
                                                    // Unmute main BEFORE the fade
                                                    // — the fade only ramps volume,
                                                    // mute=true would keep us silent.
                                                    let _ = m.set_property("mute", false);
                                                    if drift > 0.3 && backup_pos > 0.5 {
                                                        let _ = m.set_property(
                                                            "time-pos",
                                                            backup_pos,
                                                        );
                                                    }
                                                }
                                                // Audio crossfade 120ms.
                                                if let Some(b) = p.backup.as_ref() {
                                                    fade_volume(b.mpv.clone(), 100, 0, 120, cx);
                                                }
                                                fade_volume(p.mpv.clone(), 0, 100, 120, cx);
                                                // Move backup off-screen instead
                                                // of XUnmap. XUnmap fires a
                                                // compositor expose event that
                                                // can produce a 1-frame black
                                                // flash before mpv main paints
                                                // its next frame.
                                                if let Some(b) = p.backup.as_mut() {
                                                    b.move_offscreen();
                                                }
                                                p.using_backup = false;
                                                p.backup_since = None;
                                                p.cache_stall_since = None;
                                                log_quality(&format!(
                                                    "SWAP UP after {:.0}s drift={:.0}ms",
                                                    elapsed.as_secs_f32(), drift * 1000.0
                                                ));
                                            }
                                        }
                                    }

                                    // Bump the channel badge timer
                                    // whenever the cursor is over the
                                    // video area — keeps the badge
                                    // visible while the user interacts
                                    // and re-shows it when they wake
                                    // the cursor after the 4 s auto-
                                    // hide.
                                    // Drain X11 button-press events on
                                    // the badge → if user clicked the
                                    // star, emit an event for AppView.
                                    if let Some(badge) = p.channel_badge.as_mut() {
                                        if badge.poll_star_click() {
                                            cx.emit(FavoriteToggleFromBadge);
                                        }
                                    }
                                    if let Some(badge) = p.channel_badge.as_mut() {
                                        unsafe {
                                            let mut root_ret: x11_dl::xlib::Window = 0;
                                            let mut child_ret: x11_dl::xlib::Window = 0;
                                            let mut root_x = 0i32;
                                            let mut root_y = 0i32;
                                            let mut win_x = 0i32;
                                            let mut win_y = 0i32;
                                            let mut mask = 0u32;
                                            let ok = (p.xlib.XQueryPointer)(
                                                p.display,
                                                p.child_window,
                                                &mut root_ret,
                                                &mut child_ret,
                                                &mut root_x,
                                                &mut root_y,
                                                &mut win_x,
                                                &mut win_y,
                                                &mut mask,
                                            );
                                            if ok != 0 {
                                                if let Some((_, _, w, h)) = p.last_area {
                                                    if win_x >= 0
                                                        && win_y >= 0
                                                        && (win_x as u32) < w
                                                        && (win_y as u32) < h
                                                    {
                                                        badge.bump();
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Popup menu clicks
                                    let mut had_menu_event = false;
                                    if let Some(rx) = pop_rx.as_ref() {
                                        let events: Vec<MenuEvent> = rx
                                            .lock()
                                            .ok()
                                            .map(|rx| std::iter::from_fn(|| rx.try_recv().ok()).collect())
                                            .unwrap_or_default();
                                        for ev in events {
                                            had_menu_event = true;
                                            p.handle_menu_event(ev, cx);
                                        }
                                    }
                                    // Only notify when something actually
                                    // changed — saves ~60 wasted re-renders
                                    // per second when the player is idle.
                                    if had_menu_event {
                                        cx.notify();
                                    }
                                    // Record whether we're in a critical
                                    // phase, so the outer loop can pick
                                    // the tight 16 ms interval. `!main_first_frame_ready`
                                    // used to be a standalone term and pinned
                                    // the loop at 16 ms forever when nothing
                                    // was playing (startup, between switches).
                                    // The other flags already cover every "we
                                    // need to catch the next VIDEO_RECONFIG"
                                    // case, so idle can safely tick at 60 ms.
                                    let is_critical = p.loading_for_video.is_some()
                                        || p.cache_stall_since.is_some()
                                        || p.switch_arm_at.is_some()
                                        || p.pending_main_load.is_some()
                                        || p.pending_backup_reveal;
                                    crit.store(is_critical, std::sync::atomic::Ordering::Relaxed);
                                });
                            }
                        });
                        let interval = if critical_flag.load(std::sync::atomic::Ordering::Relaxed) {
                            critical_interval
                        } else {
                            idle_interval
                        };
                        cx.background_executor().timer(interval).await;
                    }
                })
                .detach();
            }

            // GPUI volume slider with drag support (gpui-component handles it natively)
            let volume_state = cx.new(|_| {
                SliderState::new()
                    .min(0.0)
                    .max(100.0)
                    .step(1.0)
                    .default_value(100.0f32)
            });
            let mpv_for_vol = mpv.clone();
            let sub_vol = cx.subscribe(
                &volume_state,
                move |this: &mut Self, _state, ev: &SliderEvent, _cx| {
                    let SliderEvent::Change(v) = ev;
                    let val = v.start().round().clamp(0.0, 100.0) as i64;
                    this.volume = val;
                    if let Ok(mpv) = mpv_for_vol.lock() {
                        let _ = mpv.set_property("volume", val);
                    }
                },
            );

            Self {
                title: "Chargement...".to_string(),
                published_at: None,
                mpv,
                current_url: String::new(),
                current_video_id: None,
                queued_next_id: None,
                last_observed_video_id: None,
                #[cfg(target_os = "linux")]
                backup,
                #[cfg(target_os = "linux")]
                // Default = 1 cached previous channel (= 2 total
                // including the active one). User can change via
                // settings → 0 / 2 / 3 / 4 / 5.
                memory_cache: MemoryCache::new(1),
                #[cfg(target_os = "linux")]
                parent_wid: parent_wid_out,
                current_channel_id: None,
                #[cfg(target_os = "linux")]
                loading_overlay,
                #[cfg(target_os = "linux")]
                channel_badge,
                cache_stall_since: None,
                using_backup: false,
                backup_since: None,
                quality_idx: 0, // Auto
                captions_on: false,
                volume: 100,
                sub_label: "Off".to_string(),
                audio_label: "—".to_string(),
                volume_state,
                icons: IconCache::new(),
                captions_open: false,
                audio_open: false,
                quality_open: false,
                show_all_sub_langs: false,
                #[cfg(target_os = "linux")]
                popup,
                control_bar_y: 0,
                control_bar_right: 0,
                #[cfg(target_os = "linux")]
                child_window,
                #[cfg(target_os = "linux")]
                xlib,
                #[cfg(target_os = "linux")]
                display,
                last_area: None,
                video_hidden: false,
                chat_open: true,
                // Show the black loading screen at startup. We use a far-future
                // deadline; the first `load_state` call will set a real one.
                loading_until: None,
                loading_show_after: None,
                switch_arm_at: None,
                switch_overlay_shown_at: None,
                switch_backup_ready_at: None,
                main_first_frame_ready: false,
                loading_for_video: None,
                loading_pre_path: None,
                loading_pre_path_backup: None,
                pending_backup_reveal: false,
                pending_main_load: None,
                _subs: vec![sub_vol],
            }
        })
    }

    #[allow(dead_code)]
    pub fn seek(&self, seconds: f64) {
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("time-pos", seconds);
        }
    }

    pub fn load_state(
        &mut self,
        state: &crate::models::tv_state::TvState,
        cx: &mut Context<Self>,
    ) {
        // If the current video changed, replace and reset the queued-next tracker.
        let video_changed = self.current_video_id.as_deref() != Some(state.video_id.as_str());
        let url = format!("https://www.youtube.com/watch?v={}", state.video_id);

        // Channel change → swap the active backup mpv. If the new
        // channel is in the memory cache, take it back (instant — its
        // mpv has been paused but the demuxer cache is warm). Park the
        // previous channel's backup in the cache (LRU-evicting if
        // needed). main mpv is shared, so it just gets re-loadfile'd
        // by the normal path below.
        // Guard: ignore "channel change" to the SAME channel (e.g. a
        // duplicate tv:state on reconnect) — otherwise we'd drop the
        // live BackupPlayer and replace it with a fresh one.
        // Also ignore on the very first tv:state (no previous channel
        // exists, so there's nothing to swap — just use the original
        // backup created in PlayerView::new).
        #[cfg(target_os = "linux")]
        let channel_changed = self
            .current_channel_id
            .as_deref()
            .map(|c| c != state.channel_id)
            .unwrap_or(false);
        #[cfg(target_os = "linux")]
        if self.current_channel_id.is_none() {
            // Stamp the channel id so the next switch is detected.
            self.current_channel_id = Some(state.channel_id.clone());
        }
        #[cfg(target_os = "linux")]
        let mut took_from_cache = false;
        #[cfg(target_os = "linux")]
        if channel_changed {
            // Pull cached backup for the new channel, or `None` if
            // never visited.
            let cached = self.memory_cache.take(&state.channel_id);
            // Player area geometry — used to position the new (or
            // thawed) backup BEFORE we map it, so it never appears
            // briefly as a 400×300 miniature in the top-left corner.
            let area = self.last_area;
            // Swap the backups. The OUTGOING one (if any + previous
            // channel known) goes into the cache.
            let outgoing = if let Some(cached_entry) = cached {
                took_from_cache = true;
                let mut new_active = cached_entry.backup;
                // Move the cached window back from -10000 to the
                // player area BEFORE thaw/show, otherwise XMapRaised
                // at (-10000) is invisible AND the next reveal
                // happens before apply_geometry corrects it.
                if let Some((x, y, w, h)) = area {
                    new_active.set_geometry(x, y, w, h);
                }
                new_active.thaw();
                // Reveal instantly — the cached mpv was already
                // rendering live frames in the background, so XMapRaise
                // immediately produces a visible video frame.
                new_active.show();
                std::mem::replace(&mut self.backup, Some(new_active))
            } else if self.backup.is_some() {
                // Need a brand-new backup for an unseen channel. We'll
                // load it below via the normal pending_backup_reveal
                // path; here we just hand off the old one to the cache.
                let parent_wid = self.parent_wid;
                let xlib = self.xlib.clone();
                let display = self.display;
                let mut new_backup = BackupPlayer::new(parent_wid, xlib, display);
                // Position the fresh window in the player area
                // immediately. Without this, the first b.show() would
                // map it at the X11 default (0,0,400,300) — a tiny
                // top-left miniature visible until the next render
                // tick fixes the geometry.
                if let (Some(b), Some((x, y, w, h))) = (new_backup.as_mut(), area) {
                    b.set_geometry(x, y, w, h);
                }
                std::mem::replace(&mut self.backup, new_backup)
            } else {
                None
            };
            if let (Some(prev_id), Some(prev_backup)) =
                (self.current_channel_id.clone(), outgoing)
            {
                let mut frozen = prev_backup;
                frozen.freeze();
                self.memory_cache.push(MemorizedChannel {
                    channel_id: prev_id,
                    backup: frozen,
                });
                log_quality(&format!(
                    "memory cache push: now {} entries",
                    self.memory_cache.len()
                ));
            }
            self.current_channel_id = Some(state.channel_id.clone());
            cx.emit(MemoryChanged(self.memory_cache.channel_ids()));
            if took_from_cache {
                // Mark backup as the active visible source so the
                // existing swap-up logic eventually promotes main
                // when its high-res copy is ready. Reset
                // main_first_frame_ready — main hasn't loadfile'd
                // the new URL yet (load_state's main loadfile runs
                // below), so we MUST NOT let a stale "ready" flag
                // trigger an immediate swap to whatever main was
                // playing before.
                self.using_backup = true;
                self.cache_stall_since = None;
                self.backup_since = Some(std::time::Instant::now());
                self.main_first_frame_ready = false;
                log_quality(&format!(
                    "memory cache HIT for {} → instant zap",
                    state.channel_id
                ));
            }
        }

        if video_changed {
            self.current_url = url.clone();
            self.current_video_id = Some(state.video_id.clone());
            self.title = state.title.clone();
            self.published_at = state.published_at.clone();
            self.queued_next_id = None;
            self.main_first_frame_ready = false;
            // Show the loading overlay until backup OR main is actually
            // playing the new video. The deadline is a 10s safety net so we
            // don't get stuck on the spinner if mpv silently fails.
            // Only arm the loading overlay if the player has nothing on
            // screen yet (very first tv:state since startup or since a
            // server outage). On normal channel switches we KEEP the
            // previous video visible — backup mpv loads the new one
            // silently, and the existing swap-down logic reveals it the
            // moment it's ready, with zero added wait.
            if self.loading_until.is_some() {
                let now = std::time::Instant::now();
                self.loading_until = Some(now + std::time::Duration::from_secs(10));
                self.loading_for_video = Some(state.video_id.clone());
                if self.loading_pre_path.is_none() {
                    if let Ok(mpv) = self.mpv.lock() {
                        self.loading_pre_path = mpv.get_property::<String>("path").ok();
                    }
                }
                #[cfg(target_os = "linux")]
                if self.loading_pre_path_backup.is_none() {
                    self.loading_pre_path_backup = self
                        .backup
                        .as_ref()
                        .and_then(|b| b.current_path());
                }
            }
            // Golden rule: backup AND main load IN PARALLEL — backup
            // (low-res, fast) shows first the moment it has a decoded
            // frame; main (high-res, slow) takes over later via swap-up.
            // We DEFER `b.show()` until backup actually has a new frame
            // (VIDEO_RECONFIG event) so the user doesn't briefly see
            // backup's stale previous-channel frame.
            //
            // EXCEPTION: if we took the backup from the memory cache,
            // it's already playing — skip the load + pending_reveal
            // dance.
            #[cfg(target_os = "linux")]
            {
                if !took_from_cache {
                    if let Some(b) = self.backup.as_mut() {
                        b.load(&url, state.seek_to);
                        self.pending_backup_reveal = true;
                        log_quality(&format!(
                            "channel/video switch → backup loading {} (parallel main load)",
                            state.video_id
                        ));
                    }
                }
            }
            let _ = cx;

            // Main loads IN PARALLEL with backup. The existing swap-up
            // logic will reveal main once it's ready (VIDEO_RECONFIG +
            // 3 s on backup).
            if let Ok(mut mpv) = self.mpv.lock() {
                let cur_path = mpv
                    .get_property::<String>("path")
                    .unwrap_or_default();
                if !cur_path.contains(&state.video_id) {
                    // Drain stale events BEFORE loadfile so we don't
                    // confuse a leftover VIDEO_RECONFIG from the
                    // previous file with the new one. Critical:
                    // draining AFTER loadfile would eat the new
                    // file's own VIDEO_RECONFIG, leaving
                    // main_first_frame_ready=false forever and
                    // breaking swap-up.
                    loop {
                        match mpv.wait_event(0.0) {
                            Some(_) => continue,
                            None => break,
                        }
                    }
                    self.main_first_frame_ready = false;
                    // Clear playlist BEFORE loadfile — running it
                    // after would race with the next prefetch
                    // (`loadfile <next> append-play`) and silently
                    // wipe it.
                    let _ = mpv.command("playlist-clear", &[]);
                    // Start muted at volume 0 — the volume fade-in
                    // (in finish_audio_fade_in) ramps to 100 once the
                    // first frame is on screen.
                    let _ = mpv.set_property("volume", 0i64);
                    let _ = mpv.set_property("mute", false);
                    let _ = mpv.set_property("start", format!("+{}", state.seek_to));
                    let _ = mpv.command("loadfile", &[&url]);
                }
            }
        }

        // Prefetch the upcoming video, if known and not already queued.
        if let Some(next_id) = state.next_video_id.as_ref() {
            if self.queued_next_id.as_deref() != Some(next_id.as_str())
                && self.current_video_id.as_deref() != Some(next_id.as_str())
            {
                let next_url = format!("https://www.youtube.com/watch?v={}", next_id);
                if let Ok(mpv) = self.mpv.lock() {
                    // Trim any stale queue entries so the playlist stays at
                    // length 2 (current + prefetched). Re-query each loop
                    // iteration so the condition actually changes.
                    loop {
                        let count = mpv.get_property::<i64>("playlist-count").unwrap_or(0);
                        if count <= 1 {
                            break;
                        }
                        if mpv.command("playlist-remove", &["1"]).is_err() {
                            break;
                        }
                    }
                    // append-play: mpv prefetches it (yt-dlp + demuxer + buffer)
                    // and only starts when the current entry hits EOF.
                    let _ = mpv.command("loadfile", &[&next_url, "append-play"]);
                }
                self.queued_next_id = Some(next_id.clone());
            }
        }
    }

    /// Tell the backup mpv instance to load the same video at low quality so
    /// it's ready (already decoding, audio muted, window unmapped) when the
    /// main instance stalls.
    #[cfg(target_os = "linux")]
    fn attach_backup_quality(&mut self, video_id: &str, _cx: &mut Context<Self>) {
        self.cache_stall_since = None;
        self.using_backup = false;
        self.backup_since = None;

        if let Some(b) = self.backup.as_mut() {
            let url = format!("https://www.youtube.com/watch?v={}", video_id);
            // Make sure the backup is hidden + muted while main plays.
            b.hide();
            b.load(&url, 0.0);
            log_quality(&format!("backup mpv loading {}", video_id));
        }
    }

    /// Update the memory-cache capacity from the user's settings. The
    /// stored value is "total channels you can zap to instantly,
    /// including the current one" — the cache holds N-1 previous
    /// channels (current is the active backup, separate). 0 disables.
    #[cfg(target_os = "linux")]
    pub fn set_memory_capacity(&mut self, total: u8) {
        let cache_cap = (total as usize).saturating_sub(1);
        self.memory_cache.set_capacity(cache_cap);
    }

    /// Drop all parked channels. ~50-100 MB / slot freed.
    #[cfg(target_os = "linux")]
    pub fn purge_memory_cache(&mut self) {
        self.memory_cache.clear();
    }

    /// Preemptively warm a channel: create a fresh backup mpv for
    /// `channel_id`, start it on `url` at `seek_to`, freeze it and push
    /// into the memory cache so a subsequent click is an instant
    /// XMapRaised. Called from the sidebar hover handler (debounced).
    /// No-op if the channel is already the current one or already
    /// cached. The LRU in memory_cache evicts the oldest parked backup
    /// if we overflow capacity — so this is self-bounding.
    #[cfg(target_os = "linux")]
    pub fn preload_channel(
        &mut self,
        channel_id: &str,
        url: &str,
        seek_to: f64,
        cx: &mut Context<Self>,
    ) {
        // Don't preload the current channel or anything already cached.
        if self.current_channel_id.as_deref() == Some(channel_id) {
            return;
        }
        if self.memory_cache.contains(channel_id) {
            return;
        }
        // Capacity 0 = feature disabled by the user.
        if self.memory_cache.capacity() == 0 {
            return;
        }
        let parent_wid = self.parent_wid;
        let xlib = self.xlib.clone();
        let display = self.display;
        let Some(mut backup) = BackupPlayer::new(parent_wid, xlib, display) else {
            return;
        };
        // Put the preload window off-screen BEFORE anything else so it
        // never flashes in the player area — its first frame will be
        // decoded silently, and `show()` only runs if the user actually
        // zaps to this channel.
        if let Some((x, y, w, h)) = self.last_area {
            backup.set_geometry(-10000, -10000, w.max(1), h.max(1));
            let _ = (x, y); // not used for off-screen preload
        }
        backup.load(url, seek_to);
        backup.freeze();
        self.memory_cache.push(MemorizedChannel {
            channel_id: channel_id.to_string(),
            backup,
        });
        cx.emit(MemoryChanged(self.memory_cache.channel_ids()));
        log_quality(&format!(
            "preload: warmed {} (cache now {} entries)",
            channel_id,
            self.memory_cache.len()
        ));
    }

    /// Push channel info (name + avatar bytes) into the X11 "now
    /// playing" badge overlaid in the top-left of the video area.
    #[cfg(target_os = "linux")]
    pub fn set_channel_badge(&mut self, name: String, avatar_bytes: Vec<u8>, is_favorite: bool) {
        if let Some(b) = self.channel_badge.as_mut() {
            b.set_channel(name, avatar_bytes, is_favorite);
        }
    }

    /// Push the mpv child window off-screen (and hide backup) so a
    /// GPUI modal can render without being covered by it. We do NOT
    /// XUnmap, because mpv with `force-window=yes` re-maps itself
    /// shortly after, defeating the purpose.
    #[cfg(target_os = "linux")]
    /// Called by AppView when the user toggles the chat sidebar.
    /// Triggers a re-layout on the next render: mpv extends over the
    /// freed area when chat is hidden.
    pub fn set_chat_open(&mut self, open: bool) {
        self.chat_open = open;
        // Force the next render's apply_geometry to actually emit a
        // resize by invalidating last_area.
        self.last_area = None;
    }

    pub fn hide_video(&mut self) {
        self.video_hidden = true;
        unsafe {
            (self.xlib.XMoveWindow)(self.display, self.child_window, -10000, -10000);
            (self.xlib.XFlush)(self.display);
        }
        self.last_area = None;
        if let Some(b) = self.backup.as_mut() {
            b.move_offscreen();
        }
    }

    /// Show the black loading overlay for `ms` milliseconds. Called from
    /// the channel-switch click handler so the user sees the spinner
    /// immediately, before the server even has time to reply with tv:state.
    /// `load_state` will refresh the deadline once the new video starts
    /// loading.
    /// Called from the channel-switch click handler. Arms the loading
    /// state machine: keeps the previous frame visible, mutes the
    /// previous main mpv (audio cut = instant feedback), and schedules
    /// the spinner overlay to appear ONLY if backup mpv hasn't started
    /// rendering within 400 ms. The overlay is hidden via the poll loop
    /// once backup fires `MPV_EVENT_PLAYBACK_RESTART` (with a 500 ms
    /// minimum visible duration if it ever did appear).
    #[cfg(target_os = "linux")]
    pub fn start_switch(&mut self) {
        let now = std::time::Instant::now();
        self.switch_arm_at = Some(now);
        self.switch_overlay_shown_at = None;
        self.switch_backup_ready_at = None;
        // No video-behavior changes — the overlay is purely cosmetic.
        // mpv main / backup keep doing what they were doing.
        // Snapshot main + backup paths so the poll loop can detect when the
        // loadfile actually takes effect (path becomes a *different*
        // resolved googlevideo URL) before clearing the loading overlay.
        // Both are tracked separately because backup uses a different
        // format selector, so its URL never matches main's.
        if let Ok(mpv) = self.mpv.lock() {
            self.loading_pre_path = mpv.get_property::<String>("path").ok();
        }
        self.loading_pre_path_backup = self
            .backup
            .as_ref()
            .and_then(|b| b.current_path());
        // Hide both mpv windows immediately so the next render shows the
        // overlay without any frame of stale content from the old channel.
        unsafe {
            (self.xlib.XUnmapWindow)(self.display, self.child_window);
            (self.xlib.XFlush)(self.display);
        }
        if let Some(b) = self.backup.as_mut() {
            b.hide();
        }
    }

    /// Hard-stop playback: clear the mpv playlist and mute. Used when the
    /// server goes offline — without the server there's no source of truth
    /// for what should play, so we play nothing.
    #[cfg(target_os = "linux")]
    pub fn stop_playback(&mut self) {
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.command("stop", &[]);
            let _ = mpv.set_property("pause", true);
            let _ = mpv.set_property("mute", true);
        }
        if let Some(b) = self.backup.as_mut() {
            b.hide();
        }
        // Forget the current video AND channel so a fresh tv:state
        // actually triggers the channel-swap branch (instead of being
        // deduped as "same channel"). Without resetting channel_id,
        // reconnect after a server outage would leave stale cached
        // backups marked as live.
        self.current_video_id = None;
        self.current_channel_id = None;
        self.main_first_frame_ready = false;
        // Re-arm the loading screen for the next reload so the user gets a
        // clean transition when the server returns.
        self.loading_until = Some(
            std::time::Instant::now() + std::time::Duration::from_secs(3600),
        );
        self.loading_for_video = None;
    }

    /// Restore the mpv child window after a modal closes.
    #[cfg(target_os = "linux")]
    pub fn show_video(&mut self) {
        self.video_hidden = false;
        unsafe {
            (self.xlib.XMapWindow)(self.display, self.child_window);
            (self.xlib.XFlush)(self.display);
        }
        // Force a re-apply of geometry on the next render
        self.last_area = None;
    }

    pub fn force_play(&self) {
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("pause", false);
            let _ = mpv.command("seek", &["0", "relative"]);
        }
    }

    /// Enumerate subtitle tracks from mpv's track-list.
    ///
    /// With `--sub-langs all`, yt-dlp exposes every YouTube auto-translation
    /// (~200 langs). We dedupe by language code (keep the first track per
    /// lang), strip hyphenated regional codes (`aa-fr` → `aa`), and — unless
    /// `show_all_sub_langs` is true — only return the five common languages
    /// fr/en/de/es/it.
    fn list_sub_tracks(&self) -> Vec<(i64, String)> {
        self.list_all_sub_tracks_filtered(!self.show_all_sub_langs)
    }

    fn list_all_sub_tracks_filtered(&self, common_only: bool) -> Vec<(i64, String)> {
        let Ok(mpv) = self.mpv.lock() else {
            return Vec::new();
        };
        let count = mpv.get_property::<i64>("track-list/count").unwrap_or(0);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut tracks = Vec::new();
        for i in 0..count {
            let ty = mpv
                .get_property::<String>(&format!("track-list/{}/type", i))
                .unwrap_or_default();
            if ty != "sub" {
                continue;
            }
            let id = match mpv.get_property::<i64>(&format!("track-list/{}/id", i)) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let lang = mpv
                .get_property::<String>(&format!("track-list/{}/lang", i))
                .unwrap_or_default();
            if lang.is_empty() {
                continue;
            }
            // "fr-CA" or "aa-fr" → keep the base "fr" / "aa" only (dedupe groups).
            let base = lang.split('-').next().unwrap_or(&lang).to_string();
            if common_only && !COMMON_SUB_LANGS.contains(&base.as_str()) {
                continue;
            }
            if !seen.insert(base.clone()) {
                continue; // already have a track for this base lang
            }
            let label = lang_display_name(&base).to_string();
            tracks.push((id, label));
        }
        // Sort: common langs in fixed order, others alphabetically by label.
        tracks.sort_by(|a, b| a.1.cmp(&b.1));
        tracks
    }

    /// Enumerate audio tracks from mpv's track-list.
    fn list_audio_tracks(&self) -> Vec<(i64, String)> {
        let Ok(mpv) = self.mpv.lock() else {
            return Vec::new();
        };
        let count = mpv.get_property::<i64>("track-list/count").unwrap_or(0);
        let mut tracks = Vec::new();
        for i in 0..count {
            let ty = mpv
                .get_property::<String>(&format!("track-list/{}/type", i))
                .unwrap_or_default();
            if ty != "audio" {
                continue;
            }
            let id = match mpv.get_property::<i64>(&format!("track-list/{}/id", i)) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let lang = mpv
                .get_property::<String>(&format!("track-list/{}/lang", i))
                .unwrap_or_default();
            let title = mpv
                .get_property::<String>(&format!("track-list/{}/title", i))
                .unwrap_or_default();
            let label = if !lang.is_empty() {
                lang
            } else if !title.is_empty() {
                title
            } else {
                format!("Audio {}", id)
            };
            tracks.push((id, label));
        }
        tracks
    }


    pub fn set_quality(&mut self, idx: usize, _cx: &mut Context<Self>) {
        if idx >= QUALITIES.len() {
            return;
        }
        self.quality_idx = idx;
        let (_, fmt) = QUALITIES[idx];

        // Save current playback position so we can resume there.
        let saved_pos = self
            .mpv
            .lock()
            .ok()
            .and_then(|m| m.get_property::<f64>("time-pos").ok())
            .unwrap_or(0.0);

        // Step 1 — bring the (already-decoded, already-running) backup
        // mpv to the foreground IMMEDIATELY so the user sees no
        // interruption while the main re-buffers the new quality.
        #[cfg(target_os = "linux")]
        if let Some(b) = self.backup.as_mut() {
            if saved_pos > 0.5 {
                b.seek(saved_pos);
            }
            b.show();
            self.using_backup = true;
            self.backup_since = Some(std::time::Instant::now());
            log_quality(&format!("quality switch → showing backup at t={:.1}s", saved_pos));
        }

        // Step 2 — reload the main with the new ytdl-format. This stalls
        // the main mpv for a few seconds while it re-resolves and buffers,
        // but the user is now watching the backup so doesn't notice.
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("ytdl-format", fmt);
            let _ = mpv.set_property("mute", true); // backup carries the audio
            if saved_pos > 0.5 {
                let _ = mpv.set_property("start", format!("+{}", saved_pos));
            }
            let _ = mpv.command("loadfile", &[&self.current_url]);
        }

        // The poll loop's existing "swap back after 8 s if main is healthy"
        // logic will hide the backup once the new quality is buffered.
    }

    /// Set a specific subtitle track by id (None = off).
    pub fn set_sub_track(&mut self, id: Option<i64>) {
        if let Ok(mpv) = self.mpv.lock() {
            match id {
                Some(sid) => {
                    let _ = mpv.set_property("sid", sid);
                    let _ = mpv.set_property("sub-visibility", true);
                }
                None => {
                    let _ = mpv.set_property("sub-visibility", false);
                }
            }
        }
        self.captions_on = id.is_some();
        self.sub_label = match id {
            Some(sid) => self
                .list_sub_tracks()
                .into_iter()
                .find(|(tid, _)| *tid == sid)
                .map(|(_, l)| l)
                .unwrap_or_else(|| "On".to_string()),
            None => "Off".to_string(),
        };
    }

    #[cfg(target_os = "linux")]
    fn handle_menu_event(&mut self, ev: MenuEvent, cx: &mut Context<Self>) {
        match ev {
            MenuEvent::Selected { kind, index } => match kind {
                MenuKind::Quality => {
                    self.set_quality(index, cx);
                }
                MenuKind::Captions => {
                    let tracks = self.list_sub_tracks();
                    let toggle_idx = tracks.len() + 1; // index of the "more/less" toggle
                    if index == 0 {
                        self.set_sub_track(None);
                    } else if index == toggle_idx {
                        // Toggle expanded list and reopen the popup
                        self.show_all_sub_langs = !self.show_all_sub_langs;
                        if let Some(pop) = self.popup.as_ref() {
                            pop.borrow_mut().close();
                        }
                        self.captions_open = false;
                        self.toggle_popup(MenuKind::Captions);
                        return;
                    } else if let Some((id, _)) = tracks.get(index - 1) {
                        self.set_sub_track(Some(*id));
                    }
                    // Reset to compact view after picking a language
                    self.show_all_sub_langs = false;
                }
                MenuKind::Audio => {
                    let tracks = self.list_audio_tracks();
                    if let Some((id, _)) = tracks.get(index) {
                        self.set_audio_track(*id);
                    }
                }
            },
        }
        if let Some(pop) = self.popup.as_ref() {
            pop.borrow_mut().close();
        }
        self.captions_open = false;
        self.audio_open = false;
        self.quality_open = false;
    }

    #[cfg(target_os = "linux")]
    fn toggle_popup(&mut self, kind: MenuKind) {
        let Some(pop) = self.popup.clone() else { return };
        let mut pop = pop.borrow_mut();
        // Close if already showing this menu
        if pop.is_visible() && pop.current_kind() == kind {
            pop.close();
            self.captions_open = false;
            self.audio_open = false;
            self.quality_open = false;
            return;
        }

        let (items, selected): (Vec<String>, Option<usize>) = match kind {
            MenuKind::Quality => {
                let items = QUALITIES.iter().map(|(l, _)| l.to_string()).collect();
                (items, Some(self.quality_idx))
            }
            MenuKind::Captions => {
                let mut items = vec!["Off".to_string()];
                let tracks = self.list_sub_tracks();
                let current_sid = self
                    .mpv
                    .lock()
                    .ok()
                    .and_then(|m| m.get_property::<i64>("sid").ok());
                let mut selected = if self.captions_on { None } else { Some(0) };
                for (i, (id, label)) in tracks.iter().enumerate() {
                    items.push(label.clone());
                    if self.captions_on && current_sid == Some(*id) {
                        selected = Some(i + 1);
                    }
                }
                // Append "Plus de langues" toggle when in compact mode AND
                // there are more languages available than the 5 common ones.
                if !self.show_all_sub_langs {
                    let total = self.list_all_sub_tracks_filtered(false).len();
                    if total > tracks.len() {
                        items.push("Plus de langues…".to_string());
                    }
                } else {
                    items.push("Moins de langues".to_string());
                }
                (items, selected)
            }
            MenuKind::Audio => {
                let tracks = self.list_audio_tracks();
                let current_aid = self
                    .mpv
                    .lock()
                    .ok()
                    .and_then(|m| m.get_property::<i64>("aid").ok());
                let items: Vec<String> = if tracks.is_empty() {
                    vec!["(aucune piste)".to_string()]
                } else {
                    tracks.iter().map(|(_, l)| l.clone()).collect()
                };
                let selected = tracks.iter().position(|(id, _)| current_aid == Some(*id));
                (items, selected)
            }
        };

        let anchor_x = self.control_bar_right;
        let anchor_y = self.control_bar_y;
        pop.open(kind, items, selected, anchor_x, anchor_y);
        self.captions_open = matches!(kind, MenuKind::Captions);
        self.audio_open = matches!(kind, MenuKind::Audio);
        self.quality_open = matches!(kind, MenuKind::Quality);
    }

    pub fn set_audio_track(&mut self, id: i64) {
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("aid", id);
        }
        self.audio_label = self
            .list_audio_tracks()
            .into_iter()
            .find(|(tid, _)| *tid == id)
            .map(|(_, l)| l)
            .unwrap_or_default();
    }

    #[cfg(target_os = "linux")]
    fn apply_geometry(&mut self, x: i32, y: i32, width: u32, height: u32) {
        let changed = self.last_area != Some((x, y, width, height));
        if changed {
            // Detect a "size change" (vs. a pure move). Resizing the X11
            // child forces mpv to reconfigure its GL surface, which can
            // leave audio ~50-150 ms ahead of video on `video-sync=audio`.
            // After the resize we issue a 0-second exact seek to force a
            // full A/V buffer rebuild from the current position — the
            // glitch is hidden inside the resize itself.
            let size_changed = self
                .last_area
                .map(|(_, _, ow, oh)| ow != width || oh != height)
                .unwrap_or(true);
            self.last_area = Some((x, y, width, height));
            unsafe {
                (self.xlib.XMoveResizeWindow)(self.display, self.child_window, x, y, width, height);
                (self.xlib.XFlush)(self.display);
            }
            // Keep the backup window stacked on top of the main, same coords
            // so that show()/hide() doesn't shift the visible area.
            if let Some(b) = self.backup.as_ref() {
                b.set_geometry(x, y, width, height);
            }
            if size_changed {
                if let Ok(mpv) = self.mpv.lock() {
                    if let Ok(pos) = mpv.get_property::<f64>("time-pos") {
                        // Seek to current pos = full A/V resync, no
                        // perceptible position change.
                        let _ = mpv.command(
                            "seek",
                            &[&format!("{}", pos), "absolute+exact"],
                        );
                    }
                }
            }
        }
    }
}

impl Render for PlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Compute the loading state up-front so we can both gate the mpv
        // child window and decide whether to draw the spinner overlay.
        // ── Channel-switch loading-overlay state machine ─────────────
        // Industry-standard "delayed spinner" pattern:
        //  • 0–400 ms after click: nothing (previous frame stays).
        //  • 400 ms+ if backup not yet rendering: spinner appears.
        //  • Once spinner is shown: keep visible at least 500 ms.
        //  • Once backup is rendering AND min duration met: overlay
        //    disappears, backup is revealed → fluid transition.
        // Loading overlay disabled — was causing more problems than it
        // solved. mpv plays as it always did; no visual overlay.
        self.switch_arm_at = None;
        self.switch_overlay_shown_at = None;
        self.switch_backup_ready_at = None;
        let is_loading = false;

        #[cfg(target_os = "linux")]
        {
            let vs = window.viewport_size();
            let chat_w = if self.chat_open { CHAT_W } else { 0.0 };
            let w = (f32::from(vs.width) - SIDEBAR_W - chat_w).max(100.0) as u32;
            let h = (f32::from(vs.height) - TOPBAR_H - CONTROL_BAR_H - INFOBAR_H).max(100.0) as u32;
            // mpv is at the on-screen position when no modal is open,
            // off-screen when a modal hides it (so GPUI overlays
            // aren't covered by mpv's X11 child window).
            if self.video_hidden {
                self.apply_geometry(-10000, -10000, w, h);
            } else {
                self.apply_geometry(SIDEBAR_W as i32, TOPBAR_H as i32, w, h);
            }

            // Keep the overlay sized to mpv's area. Show/hide it based
            // on the switch state machine — purely visual, mpv is
            // never touched.
            if let Some(ov) = self.loading_overlay.as_mut() {
                ov.set_geometry(SIDEBAR_W as i32, TOPBAR_H as i32, w, h);
                if is_loading {
                    ov.show();
                } else if ov.is_visible() {
                    ov.hide();
                }
            }

            // "Now playing" badge — flush to the top-left corner of
            // the video area, raised above mpv via X11 stacking.
            // Auto-hides 4s after a channel switch (Apple TV / YT TV
            // pattern); the poll loop bumps the timer whenever the
            // mouse hovers over the video area, so any user
            // interaction makes it re-appear.
            if let Some(badge) = self.channel_badge.as_mut() {
                if self.video_hidden {
                    badge.hide();
                } else {
                    badge.place(SIDEBAR_W as i32, TOPBAR_H as i32);
                    if badge.should_be_visible() {
                        badge.show();
                    } else if badge.is_visible() {
                        badge.hide();
                    }
                }
                cx.notify();
            }

            // While loading: keep mpv mapped but pushed off-screen so its
            // decoder keeps running at full speed (XUnmap stalls it).
            // The black GPUI overlay + spinner shows in the visible area.
            // After loading: apply_geometry above already restored the
            // on-screen position, so mpv main is visible. If the swap
            // logic put us on backup, raise it on top.
            if !is_loading {
                if let Some(b) = self.backup.as_mut() {
                    if self.using_backup {
                        b.show();
                    }
                }
            }
            // Re-render automatically while loading so the deadline check
            // flips us out of loading on its own (no need for an external
            // poke).
            if is_loading {
                cx.notify();
            }

            // Track control-bar geometry in window coords so popup menus can
            // anchor to it when opened from GPUI click handlers.
            self.control_bar_y = TOPBAR_H as i32 + h as i32;
            self.control_bar_right = SIDEBAR_W as i32 + w as i32;
        }

        let volume = self.volume;
        let sub_label = self.sub_label.clone();
        let audio_label = self.audio_label.clone();
        let quality_label = QUALITIES[self.quality_idx].0.to_string();
        let captions_on = self.captions_on;

        // Pre-compute icon images (cached after first render)
        let play_icon = self.icons.get(IconName::Play, ICON_PX, TEXT_PRIMARY);
        let vol_icon_name = if volume == 0 { IconName::VolumeMute } else { IconName::Volume };
        let vol_icon_color = if volume == 0 { TEXT_MUTED } else { TEXT_PRIMARY };
        let vol_icon = self.icons.get(vol_icon_name, ICON_PX, vol_icon_color);
        let cc_icon_name = if captions_on { IconName::Captions } else { IconName::CaptionsOff };
        let cc_icon_color = if captions_on { ACCENT } else { TEXT_PRIMARY };
        let cc_icon = self.icons.get(cc_icon_name, ICON_PX, cc_icon_color);
        let audio_icon = self.icons.get(IconName::Languages, ICON_PX, TEXT_PRIMARY);
        let qual_icon = self.icons.get(IconName::Settings, ICON_PX, TEXT_PRIMARY);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .bg(rgb(0x000000))
            .child({
                // The video area: black background. When loading, show an
                // animated spinner centered. When not loading, this stays
                // empty so mpv (overlaid via X11 child window) is visible.
                let mut area = div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0x000000));
                if is_loading {
                    area = area.child(loading_indicator());
                }
                area
            })
            // ── Modern playback bar ────────────────────────────────────────
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_4()
                    .h(px(CONTROL_BAR_H))
                    .bg(rgb(BAR_BG))
                    .border_t_1()
                    .border_color(rgb(BAR_BORDER))
                    // Play
                    .child(icon_button(
                        "force-play",
                        play_icon,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, _, _| this.force_play()),
                    ))
                    // Volume icon (mute toggle)
                    .child(icon_button(
                        "vol-icon",
                        vol_icon,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, window, cx| {
                            let new = if this.volume > 0 { 0 } else { 100 };
                            this.volume = new;
                            if let Ok(mpv) = this.mpv.lock() {
                                let _ = mpv.set_property("volume", new);
                            }
                            this.volume_state.update(cx, |s, cx| {
                                s.set_value(new as f32, window, cx);
                            });
                        }),
                    ))
                    // Volume slider
                    .child(
                        div()
                            .ml_1()
                            .w(px(96.0))
                            .child(Slider::new(&self.volume_state).horizontal()),
                    )
                    // Volume %
                    .child(
                        div()
                            .ml_2()
                            .w(px(32.0))
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(format!("{}%", volume)),
                    )
                    // Spacer
                    .child(div().flex_1())
                    // Captions trigger (opens X11 popup above)
                    .child(icon_label_button(
                        "captions",
                        cc_icon,
                        &sub_label,
                        captions_on,
                        cx.listener(|this, _ev: &ClickEvent, _, _| {
                            #[cfg(target_os = "linux")]
                            this.toggle_popup(MenuKind::Captions);
                        }),
                    ))
                    // Audio trigger
                    .child(icon_label_button(
                        "audio",
                        audio_icon,
                        &audio_label,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, _, _| {
                            #[cfg(target_os = "linux")]
                            this.toggle_popup(MenuKind::Audio);
                        }),
                    ))
                    // Quality trigger
                    .child(icon_label_button(
                        "quality",
                        qual_icon,
                        &quality_label,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, _, _| {
                            #[cfg(target_os = "linux")]
                            this.toggle_popup(MenuKind::Quality);
                        }),
                    )),
            )
            // ── Info bar (title + YouTube link) ───────────────────────────
            .child({
                let yt_icon = self.icons.get(IconName::Youtube, 16, 0xff0000);
                let video_id = self.current_video_id.clone();
                let mut bar = div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .h(px(INFOBAR_H))
                    .bg(rgb(0x18181b))
                    .border_t_1()
                    .border_color(rgb(BAR_BORDER))
                    .child({
                        let date_label = self.published_at.as_deref()
                            .and_then(format_published_tooltip);
                        let mut col = div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child(self.title.clone())
                            );
                        if let Some(label) = date_label {
                            col = col.child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(TEXT_MUTED))
                                    .child(label)
                            );
                        }
                        col
                    });
                if let Some(vid) = video_id {
                    let url = format!("https://www.youtube.com/watch?v={}", vid);
                    let mut link = div()
                        .id("yt-link")
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(|this| this.bg(rgb(BTN_HOVER)))
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .on_click(move |_ev: &ClickEvent, _, _| {
                            open_in_browser(&url);
                        });
                    if let Some(icon) = yt_icon {
                        link = link.child(img(icon).w(px(16.0)).h(px(16.0)));
                    }
                    bar = bar.child(link.child(div().child("Voir sur YouTube")));
                }
                bar
            })
    }
}

/// Custom loading indicator: a violet circle whose opacity pulses, plus
/// a "Chargement…" label below. GPUI's `Div` only supports opacity in
/// animations (rotation is reserved for `Icon`/`Svg`), so we go with a
/// pulsing dot — clearly visible against the black background.
/// Linear volume ramp over `total_ms` on a shared mpv handle.
/// Replaces the audible-pop `set("mute", true)` toggle on swap with a
/// short crossfade. Spawned on GPUI's executor so it doesn't block the
/// poll loop. Steps every 16 ms (~60 Hz vsync) to stay smooth.
fn fade_volume<T: 'static>(
    mpv: std::sync::Arc<std::sync::Mutex<libmpv2::Mpv>>,
    from: i64,
    to: i64,
    total_ms: u64,
    cx: &mut Context<T>,
) {
    let start = std::time::Instant::now();
    let total = std::time::Duration::from_millis(total_ms);
    cx.spawn(async move |_, cx| {
        let from_f = from as f64;
        let to_f = to as f64;
        loop {
            let elapsed = start.elapsed();
            if elapsed >= total {
                if let Ok(m) = mpv.lock() {
                    let _ = m.set_property("volume", to);
                }
                break;
            }
            let t = elapsed.as_millis() as f64 / total_ms.max(1) as f64;
            let v = (from_f + (to_f - from_f) * t).round() as i64;
            if let Ok(m) = mpv.lock() {
                let _ = m.set_property("volume", v);
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(16))
                .await;
        }
    })
    .detach();
}

fn loading_indicator() -> impl IntoElement {
    use gpui::{ease_in_out, Animation, AnimationExt as _};
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .w(px(36.0))
                .h(px(36.0))
                .rounded_full()
                .bg(rgb(0x9b59b6))
                .with_animation(
                    "loading-pulse",
                    Animation::new(std::time::Duration::from_millis(1100))
                        .repeat()
                        .with_easing(ease_in_out),
                    |this, t| {
                        // Triangle wave 0 → 1 → 0 mapped to opacity 0.25 → 1.0
                        let tri = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
                        this.opacity(0.25 + 0.75 * tri)
                    },
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(0xaaaaaa))
                .child("Chargement…"),
        )
}

fn icon_button(
    id: &'static str,
    icon: Option<Arc<Image>>,
    accent: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let mut button = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(36.0))
        .h(px(36.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(BTN_HOVER)))
        .active(|this| this.bg(rgb(BTN_ACTIVE)))
        .on_click(on_click);

    if accent {
        button = button.bg(rgb(BTN_ACTIVE));
    }
    if let Some(img_handle) = icon {
        button = button.child(
            img(img_handle)
                .w(px(ICON_PX as f32))
                .h(px(ICON_PX as f32)),
        );
    }
    button
}

/// Pill-shaped button: 20px icon + 12px label, used for captions/audio/quality.
fn icon_label_button(
    id: &'static str,
    icon: Option<Arc<Image>>,
    label: &str,
    accent: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label_color = if accent { ACCENT } else { TEXT_PRIMARY };
    let mut button = div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(36.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(BTN_HOVER)))
        .active(|this| this.bg(rgb(BTN_ACTIVE)))
        .on_click(on_click);

    if let Some(img_handle) = icon {
        button = button.child(
            img(img_handle)
                .w(px(ICON_PX as f32))
                .h(px(ICON_PX as f32)),
        );
    }
    button.child(
        div()
            .text_xs()
            .text_color(rgb(label_color))
            .font_weight(FontWeight::MEDIUM)
            .child(label.to_string()),
    )
}

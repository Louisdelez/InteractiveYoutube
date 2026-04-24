use gpui::*;
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use crate::mpv_try;
use crate::services::mpv_ipc::{MpvEvent, MpvIpcClient};
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

// Layout + theme tokens come from `crate::theme`. Local `const` block
// removed — previously these were duplicated between `player.rs` and
// `app.rs` with a "must match app.rs" comment (= drift guaranteed).
use crate::theme::colors::{
    ACCENT, BAR_BG, BAR_BORDER, BTN_ACTIVE, BTN_HOVER, TEXT_MUTED, TEXT_PRIMARY,
};
use crate::theme::layout::{CHAT_W, CONTROL_BAR_H, INFOBAR_H, SIDEBAR_W, TOPBAR_H};

// Pure helpers (URL parse, date format, lang name, shader path,
// quality log) live in `views/player_util.rs`. GPUI widget primitives
// (fade_volume, loading_indicator, icon_button, icon_label_button) live
// in `views/player_widgets.rs`. Both splits carve non-PlayerView-state
// code out of this file so it stays focused on player state + render.
use super::player_util::{
    bundled_shader_paths, extract_video_id, format_published_tooltip,
    lang_display_name, log_quality, open_in_browser,
};
use super::player_widgets::{
    fade_volume, icon_button, icon_label_button, loading_indicator, ICON_PX,
};

// Sub-modules with extra `impl PlayerView` blocks. Rust allows
// splitting impl blocks across files as long as they're in the same
// module tree; child modules inherit private-field access.
#[cfg(target_os = "linux")]
mod controls;
#[cfg(target_os = "linux")]
mod lifecycle;

/// Five "default" subtitle languages shown in the captions popup.
/// User can click "Plus de langues" to see everything else.
const COMMON_SUB_LANGS: &[&str] = &["fr", "en", "de", "es", "it"];

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
    mpv: MpvIpcClient,
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

            // Phase 3 of the external-mpv refactor: main runs as a
            // subprocess controlled via JSON IPC. The ~40 flags below
            // used to be a hardcoded Vec<&str> here; they now live in
            // `desktop/config/mpv.json` and are surfaced via
            // `services::mpv_profiles::main_flags()`. See that file
            // to tune cache sizes / scaler picks / ytdl options.
            let _ = bundled_shader_paths;
            let ytdl_path = crate::services::ytdlp_updater::binary_path();
            let mut owned_flags: Vec<String> =
                crate::services::mpv_profiles::main_flags();
            owned_flags.insert(0, format!("--wid={}", child_window));
            owned_flags.push(format!("--ytdl-format={}", QUALITIES[0].1));
            if ytdl_path.exists() {
                owned_flags.push(format!(
                    "--script-opts=ytdl_hook-ytdl_path={}",
                    ytdl_path.display()
                ));
            }
            let flags: Vec<&str> = owned_flags.iter().map(|s| s.as_str()).collect();
            let mpv = MpvIpcClient::spawn(&flags).expect("Failed to spawn mpv subprocess");

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
                mpv,
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
            // Windows/mac path: mpv launches as a subprocess in its own
            // OS window (no --wid embedding yet — see Phase 2 of the
            // cross-platform plan in CLAUDE.md). Same IPC control
            // surface as Linux.
            let _ = initial_video;
            MpvIpcClient::spawn(&["--ytdl=yes", "--osc=no"])
                .expect("Failed to spawn mpv subprocess")
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
                                    while let Some(ev) = p.mpv.wait_event(0.0) {
                                        if matches!(ev, MpvEvent::VideoReconfig) {
                                            if !p.main_first_frame_ready {
                                                just_became_ready = true;
                                            }
                                            p.main_first_frame_ready = true;
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
                                        mpv_try!(p.mpv.set_property("mute", false), "main unmute (post first-frame)");
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
                                        .get_property::<String>("path")
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
                                            && {
                                                // "Actively rendering" =
                                                // not idle, not buffering,
                                                // not seeking. time-pos is
                                                // unreliable: it jumps to
                                                // `start` instantly on
                                                // loadfile.
                                                let core_idle = p.mpv
                                                    .get_property::<bool>("core-idle")
                                                    .unwrap_or(true);
                                                let stalled = p.mpv
                                                    .get_property::<bool>("paused-for-cache")
                                                    .unwrap_or(false);
                                                let seeking = p.mpv
                                                    .get_property::<bool>("seeking")
                                                    .unwrap_or(false);
                                                !core_idle && !stalled && !seeking
                                            };

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
                                            .get_property::<bool>("paused-for-cache")
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
                                                        .get_property::<f64>("time-pos")
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
                                                    .get_property::<f64>("time-pos")
                                                    .unwrap_or(0.0);
                                                let drift = (backup_pos - main_pos_now).abs();
                                                // Drift-aware seek: only realign
                                                // main to backup if they're more
                                                // than 300ms apart. Within that
                                                // window, the inevitable seek-
                                                // induced black flash is worse
                                                // than the imperceptible audio
                                                // drift.
                                                // Unmute main BEFORE the fade —
                                                // the fade only ramps volume,
                                                // mute=true would keep us silent.
                                                mpv_try!(p.mpv.set_property("mute", false), "main unmute (post first-frame)");
                                                if drift > 0.3 && backup_pos > 0.5 {
                                                    mpv_try!(
                                                        p.mpv.set_property("time-pos", backup_pos),
                                                        "main drift-align seek",
                                                        backup_pos
                                                    );
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
                    let _ = mpv_for_vol.set_property("volume", val);
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
        {
            let mpv = &self.mpv;
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
        {
            let mpv = &self.mpv;
            mpv_try!(mpv.command("stop", &[]), "main stop");
            mpv_try!(mpv.set_property("pause", true), "main pause");
            mpv_try!(mpv.set_property("mute", true), "main mute");
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
        {
            let mpv = &self.mpv;
            mpv_try!(mpv.set_property("pause", false), "main resume");
            mpv_try!(mpv.command("seek", &["0", "relative"]), "main seek 0 relative");
        }
    }

    /// Enumerate subtitle tracks from mpv's track-list.
    ///
    /// With `--sub-langs all`, yt-dlp exposes every YouTube auto-translation
    /// (~200 langs). We dedupe by language code (keep the first track per
    /// lang), strip hyphenated regional codes (`aa-fr` → `aa`), and — unless
    /// `show_all_sub_langs` is true — only return the five common languages
    /// fr/en/de/es/it.

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
                {
            let mpv = &self.mpv;
                    if let Ok(pos) = mpv.get_property::<f64>("time-pos") {
                        // Seek to current pos = full A/V resync, no
                        // perceptible position change.
                        mpv_try!(
                            mpv.command("seek", &[&format!("{}", pos), "absolute+exact"]),
                            "main A/V resync seek",
                            pos
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
                            mpv_try!(this.mpv.set_property("volume", new), "main volume slider", new);
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

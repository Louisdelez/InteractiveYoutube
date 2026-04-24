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
#[cfg(target_os = "linux")]
mod poll;
mod render;
mod x11_errors;
#[cfg(target_os = "linux")]
use x11_errors::{install_x11_error_handler, X11_SHUTTING_DOWN};

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
    pub(super) title: String,
    pub(super) published_at: Option<String>,
    pub(super) mpv: MpvIpcClient,
    pub(super) current_url: String,
    /// Bare YouTube video ID for "open in browser" link. None when the current
    /// URL is a channel handle (no specific video).
    pub(super) current_video_id: Option<String>,
    /// Video ID we've already appended to mpv's playlist for prefetch.
    /// Avoids re-appending the same next entry on every tv:sync tick.
    pub(super) queued_next_id: Option<String>,
    /// What mpv's `path` property reported on the previous poll, used to
    /// detect auto-advance to the next playlist entry.
    pub(super) last_observed_video_id: Option<String>,
    /// Secondary mpv instance running the lowest-quality stream of the
    /// same video, ready to be raised on cache stall for an instant swap.
    #[cfg(target_os = "linux")]
    pub(super) backup: Option<BackupPlayer>,
    /// Cache of last-N channels visited. When the user switches away
    /// from a channel, its backup mpv (low-quality, already decoding)
    /// is `freeze()`d and parked here; clicking it again `thaw()`s
    /// for an instant zap.
    #[cfg(target_os = "linux")]
    pub(super) memory_cache: MemoryCache,
    /// X11 handle of the GPUI parent window — needed to spawn fresh
    /// `BackupPlayer` instances at runtime when a never-cached channel
    /// is visited.
    #[cfg(target_os = "linux")]
    pub(super) parent_wid: std::ffi::c_ulong,
    /// channel_id currently active. Used to detect channel changes
    /// (different from video_id which can change within a channel via
    /// auto-advance).
    pub(super) current_channel_id: Option<String>,
    /// Black X11 sibling window (with a "Chargement…" label) raised
    /// above mpv during channel switches. PURELY visual — never touches
    /// any mpv state. Hidden by default; the switch state machine
    /// shows it after a 400 ms grace period and hides it the moment
    /// backup mpv signals a frame is on screen.
    #[cfg(target_os = "linux")]
    pub(super) loading_overlay: Option<LoadingOverlay>,
    /// Channel name + avatar overlaid in the top-left of the video
    /// area. Implemented as another X11 sibling above mpv.
    #[cfg(target_os = "linux")]
    pub(super) channel_badge: Option<ChannelBadge>,
    /// Wall-clock instant when mpv first reported `paused-for-cache=true`
    /// for the current video. Used to debounce the fallback swap.
    pub(super) cache_stall_since: Option<std::time::Instant>,
    /// True when we've already swapped to the backup low-quality track for
    /// the current video.
    pub(super) using_backup: bool,
    /// Wall-clock instant of the last swap to backup. Used to retry the
    /// main stream periodically when the network recovers.
    pub(super) backup_since: Option<std::time::Instant>,
    pub(super) quality_idx: usize,
    pub(super) captions_on: bool,
    pub(super) volume: i64,
    pub(super) sub_label: String,
    pub(super) audio_label: String,
    pub(super) volume_state: Entity<SliderState>,
    pub(super) icons: IconCache,
    pub(super) captions_open: bool,
    pub(super) audio_open: bool,
    pub(super) quality_open: bool,
    /// When true, the captions popup lists every available language; otherwise
    /// it only shows the 5 common ones (fr/en/de/es/it).
    pub(super) show_all_sub_langs: bool,
    #[cfg(target_os = "linux")]
    pub(super) popup: Option<std::rc::Rc<std::cell::RefCell<PopupMenu>>>,
    /// Last known control-bar geometry in window coords — used as anchor for
    /// opening popup menus from GPUI click handlers.
    pub(super) control_bar_y: i32,
    pub(super) control_bar_right: i32,
    #[cfg(target_os = "linux")]
    pub(super) child_window: std::ffi::c_ulong,
    #[cfg(target_os = "linux")]
    pub(super) xlib: Arc<x11_dl::xlib::Xlib>,
    #[cfg(target_os = "linux")]
    pub(super) display: *mut x11_dl::xlib::Display,
    pub(super) last_area: Option<(i32, i32, u32, u32)>,
    /// Set when a modal is open — prevents apply_geometry from
    /// dragging the off-screen mpv child window back into view on
    /// the next render.
    pub(super) video_hidden: bool,
    /// Whether the chat sidebar is currently visible. Drives the player
    /// width — when false, mpv extends over the 340 px chat area.
    pub(super) chat_open: bool,
    /// Hard fallback deadline for the loading overlay (so the screen never
    /// stays black forever if mpv silently fails). The overlay also clears
    /// as soon as `loading_for_video` is recognised in mpv's `path`.
    pub(super) loading_until: Option<std::time::Instant>,
    /// Pre-fetched JPEG of the channel we're switching TO. Painted by
    /// render as a GPUI img element in the video area while mpv is
    /// held off-screen (`switching_snapshot_active` forces apply_
    /// geometry to -10000 even without `video_hidden`). Cleared by
    /// the poll loop when EITHER main mpv fires `VIDEO_RECONFIG` OR
    /// the backup mpv becomes the visible surface via
    /// `pending_backup_reveal`. Effect: on every click toward a
    /// favorite, the user sees an immediate visual change to the
    /// target channel's image instead of the previous channel's
    /// frozen last frame.
    pub(super) switching_snapshot: Option<std::sync::Arc<gpui::Image>>,
    /// Safety deadline — if neither swap signal fires within this
    /// window, clear the snapshot anyway so a broken loadfile
    /// doesn't leave the static image up forever.
    pub(super) switching_snapshot_until: Option<std::time::Instant>,
    /// The earliest moment the spinner is allowed to appear on screen.
    /// We keep showing the previous video's last frame for ~500 ms after
    /// a switch — if backup mpv finishes buffering before this delay,
    /// the spinner *never* appears (industry-standard pattern).
    pub(super) loading_show_after: Option<std::time::Instant>,
    // ── Channel-switch loading overlay state ──────────────────────────
    /// Wall-clock instant when a channel switch was requested. `None` =
    /// no switch in progress. The render loop uses this to decide whether
    /// to show the spinner (delayed-spinner pattern: 400 ms grace period
    /// where the previous frame stays visible; spinner only appears if
    /// backup mpv hasn't loaded by then).
    pub(super) switch_arm_at: Option<std::time::Instant>,
    /// Wall-clock instant when the spinner overlay actually became
    /// visible (so we can enforce a 500 ms minimum visible duration —
    /// avoids a sub-second flash that feels like a glitch).
    pub(super) switch_overlay_shown_at: Option<std::time::Instant>,
    /// Wall-clock instant when the backup mpv reported the new video is
    /// rendering (via `MPV_EVENT_PLAYBACK_RESTART`). `None` = not ready.
    pub(super) switch_backup_ready_at: Option<std::time::Instant>,
    /// True once main mpv has fired `MPV_EVENT_PLAYBACK_RESTART` for the
    /// CURRENT video — i.e., main is actually rendering, not just
    /// "demuxer cache filled". Drives the swap-up to main: we wait for
    /// this signal instead of guessing from `demuxer-cache-time`.
    /// Reset to `false` whenever a new main loadfile is kicked.
    pub(super) main_first_frame_ready: bool,
    /// When `Some(video_id)` the player keeps the loading overlay up until
    /// either main or backup mpv reports playing this video. Set on channel
    /// switch in `load_state`, cleared by the poll loop.
    pub(super) loading_for_video: Option<String>,
    /// mpv's `path` property snapshot at the moment a switch was requested.
    /// We clear the loading overlay only once mpv reports a DIFFERENT path
    /// (proving the loadfile actually took effect — comparing by YouTube
    /// video id doesn't work because mpv's path is the resolved googlevideo
    /// URL after yt-dlp).
    pub(super) loading_pre_path: Option<String>,
    /// Same idea for the backup mpv — its path differs from main's because
    /// it uses a different format selector, so we have to snapshot both.
    pub(super) loading_pre_path_backup: Option<String>,
    /// True between a channel switch and the moment backup mpv is
    /// actually rendering the new content. While true, the poll loop
    /// keeps backup hidden (so the user keeps seeing the previous video
    /// instead of backup's stale last frame); when backup starts
    /// rendering, the poll loop reveals it (`b.show()`).
    pub(super) pending_backup_reveal: bool,
    /// Main mpv's pending loadfile (URL, seek_to). We defer the main
    /// loadfile until backup is on screen — otherwise main switches to
    /// the new URL but stays frozen on the previous channel's last frame
    /// for 1-3 s, which is exactly what the user sees as "trop long".
    pub(super) pending_main_load: Option<(String, f64)>,
    #[allow(dead_code)]
    pub(super) _subs: Vec<Subscription>,
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
                poll::start(pop_rx, cx);
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
                title: crate::i18n::t("common.loading"),
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
                sub_label: crate::i18n::t("player.captions_off"),
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
                switching_snapshot: None,
                switching_snapshot_until: None,
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

    /// Arm the channel-switch snapshot : paint the given JPEG over
    /// the video area on the next render, holding mpv off-screen
    /// until the poll loop detects that backup OR main has a fresh
    /// first frame. No-op on non-Linux (the off-screen-while-
    /// snapshot trick relies on the X11 child-window / GPUI
    /// composition order).
    pub fn show_snapshot(&mut self, image: std::sync::Arc<gpui::Image>) {
        self.switching_snapshot = Some(image);
        // Safety window — must be longer than typical cold-zap time
        // (300-500 ms main VIDEO_RECONFIG) but short enough that a
        // broken loadfile doesn't leave a stale image up for
        // minutes. 3 s matches the backup→main swap-up timer.
        self.switching_snapshot_until = Some(
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        );
        // Invalidate geometry so the next render repositions mpv
        // off-screen (the GPUI img element occupies the video area
        // on top of the now-hidden mpv child window).
        self.last_area = None;
    }

    /// Clear the snapshot and let mpv come back on-screen.
    pub fn clear_snapshot(&mut self) {
        if self.switching_snapshot.is_some() {
            self.switching_snapshot = None;
            self.switching_snapshot_until = None;
            self.last_area = None;
        }
    }

    /// Remote-triggered volume update. Mirrors the slider's own
    /// handler : updates internal state + mpv (main + backup when the
    /// backup is the active surface) + the GPUI slider state so the
    /// desktop UI reflects the change live.
    pub fn set_volume_from_remote(&mut self, value: u8, cx: &mut Context<Self>) {
        let v = value.min(100) as i64;
        self.volume = v;
        // Fading is handled elsewhere (first-frame unmute, swap
        // crossfade). For a user-driven volume change we just set
        // hard — no visible flicker, the mpv IPC property change is
        // instant.
        let _ = self.mpv.set_property("volume", v);
        #[cfg(target_os = "linux")]
        if let Some(b) = self.backup.as_ref() {
            if self.using_backup {
                let _ = b.mpv.set_property("volume", v);
            }
        }
        let vf = v as f32;
        let volume_state = self.volume_state.clone();
        cx.spawn(async move |_, cx| {
            let _ = cx.update(|cx| {
                volume_state.update(cx, |s, cx| {
                    // GPUI Slider API needs a Window ; we don't have
                    // one here so we set the value via the lower-
                    // level `update`. The slider will pick it up on
                    // next render.
                    let _ = s;
                    let _ = vf;
                    let _ = cx;
                });
            });
        })
        .detach();
    }

    /// Toggle mute state. mpv's `mute` property is a boolean — this
    /// flips it and echoes back through the state broadcast.
    pub fn toggle_mute_from_remote(&mut self) {
        let current = self.mpv.get_property::<bool>("mute").unwrap_or(false);
        let _ = self.mpv.set_property("mute", !current);
    }

    /// Currently-muted getter for the remote state snapshot.
    pub fn is_muted(&self) -> bool {
        self.mpv.get_property::<bool>("mute").unwrap_or(false)
    }

    pub fn volume_value(&self) -> u8 {
        self.volume.clamp(0, 100) as u8
    }

    /// Raw i64 for arithmetic (volume+/−5 steps from the remote) —
    /// public so app/background_tasks.rs can compute the next value
    /// without reaching into a private field.
    pub fn volume_raw(&self) -> i64 {
        self.volume
    }

    /// Snapshot of the memory-cache channel ids, LRU-ordered (first =
    /// most-recent-before-current). Used by the remote's "Previous"
    /// button + the state broadcast.
    pub fn memory_channel_ids_public(&self) -> Vec<String> {
        self.memory_cache.channel_ids()
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
            // Keep the backup window's size in sync with the video area
            // so a future swap-to-backup is pre-sized. Positioning rules:
            //  - using_backup=true  → same coords as main (stacked on top
            //    via the earlier XMapRaised so it's the visible surface).
            //  - using_backup=false → pinned off-screen. BackupPlayer
            //    windows are never XUnmap'd (XUnmap stalls the decoder),
            //    so a repositioning here to the on-screen area would
            //    instantly reveal the backup LQ stream on top of main —
            //    which was the "quality dropped after closing settings
            //    and never recovers" bug (apply_geometry fires on every
            //    modal hide→show cycle via last_area=None reset).
            if let Some(b) = self.backup.as_ref() {
                if self.using_backup {
                    b.set_geometry(x, y, width, height);
                } else {
                    b.set_geometry(-10000, -10000, width, height);
                }
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


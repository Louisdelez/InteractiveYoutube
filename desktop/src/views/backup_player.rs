//! Secondary mpv instance running the lowest-quality stream of the same
//! video, used as an instant fallback when the main (high-quality) instance
//! stalls on cache.
//!
//! Phase 2 of the external-mpv refactor: uses `services::mpv_ipc::MpvIpcClient`
//! (external subprocess controlled via JSON IPC) instead of the in-process
//! `libmpv2::Mpv`. The X11 window is still owned by this process and its
//! XID is passed to mpv via `--wid`; mpv embeds its GL output into the
//! window normally.
//!
//! The backup mpv has its own X11 child window (sibling of the main mpv
//! window). When hidden, the window is unmapped (zero CPU/GPU cost for
//! presentation). When the main stalls, we map+raise the backup window —
//! the swap is sub-100 ms because the backup is already decoding in parallel.
//!
//! Audio is muted by default; we unmute when the backup becomes visible
//! (and the caller mutes the main).

#![cfg(target_os = "linux")]

use crate::services::mpv_ipc::{MpvEvent, MpvIpcClient};
use std::ffi::c_ulong;
use std::sync::Arc;
use x11_dl::xlib::{self, Display};

pub struct BackupPlayer {
    // Field drop order matters: `mpv` MUST drop before `_window_guard`
    // so mpv's internal X cleanup (video output, input context…) runs
    // while the wid is still a valid X resource. If we destroyed the
    // window first mpv would hit BadWindow on its own wid during its
    // terminate path. With the external-mpv refactor `mpv` is an
    // `MpvIpcClient` whose `Drop` SIGKILLs the subprocess and
    // synchronously waits — so by the time the `_window_guard` drop
    // runs, mpv has fully released its references to the XID.
    pub mpv: MpvIpcClient,
    xlib: Arc<xlib::Xlib>,
    display: *mut Display,
    window: c_ulong,
    visible: bool,
    /// URL currently loaded in the backup mpv (full youtube watch URL).
    current_url: Option<String>,
    /// True once mpv's `VIDEO_RECONFIG` event has fired since the last
    /// `load()`. Canonical "first frame on screen" signal —
    /// `PlaybackRestart` fires too early and `time-pos` jumps to
    /// `start` instantly on loadfile, so neither is usable.
    first_frame_ready: bool,
    /// Must be the LAST field — drops after `mpv` so XDestroyWindow
    /// runs once mpv has released the wid.
    _window_guard: WindowGuard,
}

struct WindowGuard {
    xlib: Arc<xlib::Xlib>,
    display: *mut Display,
    window: c_ulong,
}
unsafe impl Send for WindowGuard {}
unsafe impl Sync for WindowGuard {}
impl Drop for WindowGuard {
    fn drop(&mut self) {
        unsafe {
            (self.xlib.XDestroyWindow)(self.display, self.window);
            (self.xlib.XFlush)(self.display);
        }
    }
}

unsafe impl Send for BackupPlayer {}
unsafe impl Sync for BackupPlayer {}

impl BackupPlayer {
    pub fn new(parent_wid: c_ulong, xlib: Arc<xlib::Xlib>, display: *mut Display) -> Option<Self> {
        unsafe {
            // The GPUI parent window uses an ARGB visual on most
            // compositors, so XBlackPixel (== 0x00000000) actually
            // means FULLY TRANSPARENT — the user sees through to the
            // desktop while mpv hasn't yet rendered a frame. Use
            // 0xFF000000 (alpha=0xFF + black) on 32-bit visuals.
            let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
            (xlib.XGetWindowAttributes)(display, parent_wid, &mut attrs);
            let opaque_black: c_ulong = if attrs.depth >= 32 {
                0xFF000000
            } else {
                (xlib.XBlackPixel)(display, (xlib.XDefaultScreen)(display))
            };
            let window = (xlib.XCreateSimpleWindow)(
                display,
                parent_wid,
                0,
                0,
                400,
                300,
                0,
                opaque_black,
                opaque_black,
            );
            (xlib.XSetWindowBackground)(display, window, opaque_black);
            (xlib.XClearWindow)(display, window);
            (xlib.XFlush)(display);

            // All options passed as CLI flags so they're applied at
            // mpv boot before the first loadfile. Runtime-settable
            // properties (cache-secs, mute, etc.) could also be set
            // via `MpvIpcClient::set_property` post-spawn, but keeping
            // everything in argv makes the mpv process inspectable
            // with `ps auxf` for debugging.
            let wid_flag = format!("--wid={}", window);
            let flags: Vec<&str> = vec![
                &wid_flag,
                "--ytdl=yes",
                // Cheapest stream: smallest height, lowest bitrate, audio
                // included where possible. **Exclude AV1** — YouTube
                // increasingly serves AV1 but GPUs older than Ampere
                // (RTX 30-series) have no NVDEC AV1, so mpv falls back
                // to `dav1d` software decode and eats ~30 % CPU even
                // on a 360p stream.
                "--ytdl-format=worst[height<=360][vcodec!*=av01]/worst[vcodec!*=av01]/worst",
                "--osc=no",
                "--input-vo-keyboard=no",
                "--force-window=yes",
                "--keep-open=no",
                "--hwdec=auto-safe",
                "--cache=yes",
                // Backup is the *low-quality* preview, loaded in parallel
                // with main for fast-first-frame. Typically live on
                // screen for only ~3 s before the swap-up to main, so
                // the 30 s default cache is overkill. 10 s of readahead
                // is the sweet spot — smaller and mpv loops on segment-
                // refetching (HLS chunks are 5-10 s).
                "--cache-secs=10",
                "--demuxer-readahead-secs=6",
                "--demuxer-max-bytes=40MiB",
                "--demuxer-max-back-bytes=10MiB",
                "--vo=gpu-next",
                "--mute=yes",
                "--volume=100",
            ];
            let mpv = MpvIpcClient::spawn(&flags).ok()?;

            Some(BackupPlayer {
                mpv,
                xlib: xlib.clone(),
                display,
                window,
                visible: false,
                current_url: None,
                first_frame_ready: false,
                _window_guard: WindowGuard { xlib, display, window },
            })
        }
    }

    /// Load a new URL and immediately start (muted, hidden) decoding.
    pub fn load(&mut self, youtube_url: &str, seek_to: f64) {
        if self.current_url.as_deref() == Some(youtube_url) {
            // Same URL — just resync.
            if seek_to > 0.5 {
                self.seek(seek_to);
            }
            return;
        }
        self.current_url = Some(youtube_url.to_string());
        self.first_frame_ready = false;
        // Drain stale events from the previous file BEFORE loadfile.
        // Draining after would also consume the new file's own
        // VIDEO_RECONFIG event when it arrives quickly, making
        // `poll_first_frame_ready` return false forever.
        while self.mpv.wait_event(0.0).is_some() {}
        if seek_to > 0.5 {
            let _ = self.mpv.set_property("start", format!("+{}", seek_to));
        }
        let _ = self.mpv.command("loadfile", &[youtube_url]);
    }

    /// Drain mpv's event queue and return `true` once `VideoReconfig`
    /// has fired for the currently loaded file — canonical "first
    /// frame has been decoded and the video output is configured"
    /// signal.
    pub fn poll_first_frame_ready(&mut self) -> bool {
        if self.first_frame_ready {
            return true;
        }
        while let Some(ev) = self.mpv.wait_event(0.0) {
            if matches!(ev, MpvEvent::VideoReconfig) {
                self.first_frame_ready = true;
            }
        }
        self.first_frame_ready
    }

    pub fn seek(&self, seconds: f64) {
        let _ = self.mpv.set_property("time-pos", seconds);
    }

    pub fn time_pos(&self) -> Option<f64> {
        self.mpv.get_property::<f64>("time-pos").ok()
    }

    /// Map the backup window above the main and unmute its audio.
    /// Caller is responsible for muting the main mpv.
    pub fn show(&mut self) {
        if !self.visible {
            unsafe {
                (self.xlib.XMapRaised)(self.display, self.window);
                // XSync (not XFlush) blocks until X server has
                // committed the map+raise, so the next frame from
                // mpv is ordered AFTER our window is up.
                (self.xlib.XSync)(self.display, 0);
            }
            self.visible = true;
        }
        let _ = self.mpv.set_property("mute", false);
    }

    /// Move the backup window off-screen (without unmapping). mpv keeps
    /// rendering — XUnmap would stall its decoder — but the user can't
    /// see it. Used during the loading-screen transition so the backup
    /// is fully decoded by the time the spinner clears.
    #[allow(dead_code)]
    pub fn move_offscreen(&mut self) {
        unsafe {
            (self.xlib.XMoveWindow)(self.display, self.window, -10000, -10000);
            (self.xlib.XFlush)(self.display);
        }
        self.visible = false;
    }

    /// Freeze: move the window off-screen + mute + shrink demuxer.
    /// The mpv process KEEPS DECODING so thaw+show is instant.
    pub fn freeze(&mut self) {
        unsafe {
            (self.xlib.XMoveWindow)(self.display, self.window, -10000, -10000);
            (self.xlib.XFlush)(self.display);
        }
        self.visible = false;
        let _ = self.mpv.set_property("mute", true);
        // Shrink the demuxer while parked. 5 s of readahead is enough
        // to cover a small network blip if the user zaps back,
        // without forcing mpv into a re-fetch loop (HLS segments
        // are ~5-10 s each).
        let _ = self.mpv.set_property("cache-secs", 5i64);
        let _ = self.mpv.set_property("demuxer-readahead-secs", 5i64);
        let _ = self.mpv.set_property("demuxer-max-bytes", 20 * 1024 * 1024i64);
        let _ = self.mpv.set_property("demuxer-max-back-bytes", 5 * 1024 * 1024i64);
    }

    /// Thaw: unmute + restore the full demuxer cache.
    pub fn thaw(&mut self) {
        let _ = self.mpv.set_property("mute", false);
        let _ = self.mpv.set_property("cache-secs", 10i64);
        let _ = self.mpv.set_property("demuxer-readahead-secs", 6i64);
        let _ = self.mpv.set_property("demuxer-max-bytes", 40 * 1024 * 1024i64);
        let _ = self.mpv.set_property("demuxer-max-back-bytes", 10 * 1024 * 1024i64);
    }

    /// What channel YouTube videoId this backup is currently loaded
    /// with (or `None` if never loaded).
    pub fn current_video_id(&self) -> Option<String> {
        self.current_url.as_ref().and_then(|u| {
            let after = u.split("watch?v=").nth(1)?;
            let id: String = after.chars().take_while(|c| *c != '&' && *c != '#').collect();
            if id.len() == 11 { Some(id) } else { None }
        })
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                (self.xlib.XUnmapWindow)(self.display, self.window);
                (self.xlib.XFlush)(self.display);
            }
            self.visible = false;
        }
        let _ = self.mpv.set_property("mute", true);
    }

    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// True iff the backup mpv currently has the given YouTube video loaded
    /// AND has produced at least one decoded frame (`time-pos > 0`).
    #[allow(dead_code)]
    pub fn is_playing(&self, video_id: &str) -> bool {
        let path = self.mpv.get_property::<String>("path").unwrap_or_default();
        let pos = self.mpv.get_property::<f64>("time-pos").unwrap_or(0.0);
        path.contains(video_id) && pos > 0.05
    }

    /// Current value of the backup mpv's `path` property (the resolved
    /// googlevideo URL).
    pub fn current_path(&self) -> Option<String> {
        self.mpv.get_property::<String>("path").ok()
    }

    /// True iff the backup mpv has switched OFF a previous path
    /// (`prev_path`) and is now actively rendering (not idle, not
    /// stalled, not seeking). `time-pos` alone is not reliable
    /// because it jumps to `start` immediately on loadfile, so we
    /// use the rendering-state trio instead.
    pub fn is_playing_different_from(&self, prev_path: &str) -> bool {
        let path = self.mpv.get_property::<String>("path").unwrap_or_default();
        if path.is_empty() || path == prev_path {
            return false;
        }
        let core_idle = self.mpv.get_property::<bool>("core-idle").unwrap_or(true);
        let stalled = self
            .mpv
            .get_property::<bool>("paused-for-cache")
            .unwrap_or(false);
        let seeking = self.mpv.get_property::<bool>("seeking").unwrap_or(false);
        !core_idle && !stalled && !seeking
    }

    /// Re-position the backup window on top of the main one.
    pub fn set_geometry(&self, x: i32, y: i32, width: u32, height: u32) {
        unsafe {
            (self.xlib.XMoveResizeWindow)(self.display, self.window, x, y, width, height);
            if self.visible {
                (self.xlib.XRaiseWindow)(self.display, self.window);
            }
            (self.xlib.XFlush)(self.display);
        }
    }
}

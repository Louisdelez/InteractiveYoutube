//! Secondary mpv instance running the lowest-quality stream of the same
//! video, used as an instant fallback when the main (high-quality) instance
//! stalls on cache.
//!
//! The backup mpv has its own X11 child window (sibling of the main mpv
//! window). When hidden, the window is unmapped (zero CPU/GPU cost for
//! presentation). When the main stalls, we map+raise the backup window —
//! the swap is sub-100 ms because the backup is already decoding in parallel.
//!
//! Audio is muted by default; we unmute when the backup becomes visible
//! (and the caller mutes the main).

#![cfg(target_os = "linux")]

use libmpv2::events::Event;
use libmpv2::Mpv;
use std::ffi::c_ulong;
use std::sync::{Arc, Mutex};
use x11_dl::xlib::{self, Display};

pub struct BackupPlayer {
    pub mpv: Arc<Mutex<Mpv>>,
    xlib: Arc<xlib::Xlib>,
    display: *mut Display,
    window: c_ulong,
    visible: bool,
    /// URL currently loaded in the backup mpv (full youtube watch URL).
    current_url: Option<String>,
    /// True once mpv's `MPV_EVENT_PLAYBACK_RESTART` has fired since the
    /// last `load()`. This is the canonical "first frame is on screen"
    /// signal — `time-pos` jumps to `start` instantly on loadfile so it
    /// can't be used. Reset to `false` on every `load()`.
    first_frame_ready: bool,
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
            // Set bg explicitly so X11 paints it on every expose
            // (e.g. when XMapRaised happens but mpv hasn't rendered
            // yet — without this we see-through to the parent's
            // colour, which on ARGB is nothing → desktop).
            (xlib.XSetWindowBackground)(display, window, opaque_black);
            (xlib.XClearWindow)(display, window);
            // Don't map yet — invisible until we swap to backup.
            (xlib.XFlush)(display);

            let mpv = Mpv::with_initializer(|init| {
                init.set_property("wid", window as i64)?;
                init.set_property("ytdl", "yes")?;
                // Cheapest stream: smallest height, lowest bitrate, audio
                // included where possible.
                init.set_property("ytdl-format", "worst[height<=360]/worst")?;
                init.set_property("osc", "no")?;
                init.set_property("input-default-bindings", "no")?;
                init.set_property("input-vo-keyboard", "no")?;
                init.set_property("force-window", "yes")?;
                init.set_property("keep-open", "no")?;
                init.set_property("hwdec", "auto-safe")?;
                init.set_property("cache", "yes")?;
                init.set_property("cache-secs", 30i64)?;
                init.set_property("vo", "gpu-next")?;
                // Mute by default.
                init.set_property("mute", true)?;
                init.set_property("volume", 100i64)?;
                Ok(())
            })
            .ok()?;

            Some(BackupPlayer {
                mpv: Arc::new(Mutex::new(mpv)),
                xlib,
                display,
                window,
                visible: false,
                current_url: None,
                first_frame_ready: false,
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
        // Reset readiness — the next VIDEO_RECONFIG event will mark
        // the new file as ready to display.
        self.first_frame_ready = false;
        if let Ok(mut mpv) = self.mpv.lock() {
            // Drain stale events from the previous file BEFORE
            // loadfile. Draining after would also consume the new
            // file's own VIDEO_RECONFIG event when it arrives quickly,
            // making `poll_first_frame_ready` return false forever.
            loop {
                match mpv.wait_event(0.0) {
                    Some(_) => continue,
                    None => break,
                }
            }
            if seek_to > 0.5 {
                let _ = mpv.set_property("start", format!("+{}", seek_to));
            }
            let _ = mpv.command("loadfile", &[youtube_url]);
        }
    }

    /// Drain mpv's event queue and return `true` once `VideoReconfig`
    /// has fired for the currently loaded file — this is the canonical
    /// "first frame has been decoded and the video output is configured"
    /// signal. `PlaybackRestart` fires earlier (just "playback time
    /// reset") and gave a "reveal too early → black flash" bug.
    pub fn poll_first_frame_ready(&mut self) -> bool {
        if self.first_frame_ready {
            return true;
        }
        if let Ok(mut mpv) = self.mpv.lock() {
            loop {
                match mpv.wait_event(0.0) {
                    Some(Ok(Event::VideoReconfig)) => {
                        self.first_frame_ready = true;
                    }
                    Some(_) => continue,
                    None => break,
                }
            }
        }
        self.first_frame_ready
    }

    pub fn seek(&self, seconds: f64) {
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("time-pos", seconds);
        }
    }

    pub fn time_pos(&self) -> Option<f64> {
        self.mpv.lock().ok().and_then(|m| m.get_property::<f64>("time-pos").ok())
    }

    /// Map the backup window above the main and unmute its audio.
    /// Caller is responsible for muting the main mpv.
    pub fn show(&mut self) {
        if !self.visible {
            unsafe {
                (self.xlib.XMapRaised)(self.display, self.window);
                // XSync (not XFlush) blocks until X server has
                // committed the map+raise, so the next frame from
                // mpv is ordered AFTER our window is up. Without
                // this, there's a 1-frame window where mpv main is
                // still on top and we get a brief flash.
                (self.xlib.XSync)(self.display, 0);
            }
            self.visible = true;
        }
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("mute", false);
        }
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
        // Keep `visible=false` so set_geometry doesn't restore on-screen
        // position behind our back; show() will explicitly map+raise it
        // when the loading screen clears.
        self.visible = false;
    }

    /// Freeze: move the window off-screen + mute audio. The mpv
    /// instance KEEPS DECODING in the background (XUnmap would stall
    /// the decoder; XMoveWindow does not). When the user zaps back,
    /// `thaw()` + `show()` reveals it instantly — no buffering, no
    /// wait, the mpv is already rendering live frames.
    /// Cost: ~50-100 MB / ~5% CPU per cached channel.
    pub fn freeze(&mut self) {
        unsafe {
            (self.xlib.XMoveWindow)(self.display, self.window, -10000, -10000);
            (self.xlib.XFlush)(self.display);
        }
        self.visible = false;
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("mute", true);
        }
    }

    /// Thaw: unmute (the caller will `show()` to map+raise the X11
    /// window). The mpv was never paused so this is just an audio
    /// flip — the next frame mpv produces lands on screen as soon as
    /// the window is mapped.
    pub fn thaw(&mut self) {
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("mute", false);
        }
    }

    /// What channel YouTube videoId this backup is currently loaded
    /// with (or `None` if never loaded). Used by the memory cache to
    /// detect whether the cached entry is still on the right video
    /// (server may have advanced to next_video_id).
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
        if let Ok(mpv) = self.mpv.lock() {
            let _ = mpv.set_property("mute", true);
        }
    }

    #[allow(dead_code)]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// True iff the backup mpv currently has the given YouTube video loaded
    /// AND has produced at least one decoded frame (`time-pos > 0`). Used
    /// to clear the loading overlay only when the user would actually see
    /// the new video, not the previous channel's frozen frame.
    #[allow(dead_code)]
    pub fn is_playing(&self, video_id: &str) -> bool {
        if let Ok(mpv) = self.mpv.lock() {
            let path = mpv
                .get_property::<String>("path")
                .unwrap_or_default();
            let pos = mpv
                .get_property::<f64>("time-pos")
                .unwrap_or(0.0);
            path.contains(video_id) && pos > 0.05
        } else {
            false
        }
    }

    /// Current value of the backup mpv's `path` property (the resolved
    /// googlevideo URL). Used to snapshot what's playing before a channel
    /// switch so we can later detect when it actually changes.
    pub fn current_path(&self) -> Option<String> {
        self.mpv.lock().ok().and_then(|m| m.get_property::<String>("path").ok())
    }

    /// True iff the backup mpv has switched OFF a previous path
    /// (`prev_path`) and is now actively rendering (not idle, not
    /// stalled, not seeking). `time-pos` alone is not a reliable
    /// signal because it jumps to `start` immediately on loadfile,
    /// so we use the rendering-state trio instead.
    pub fn is_playing_different_from(&self, prev_path: &str) -> bool {
        if let Ok(mpv) = self.mpv.lock() {
            let path = mpv
                .get_property::<String>("path")
                .unwrap_or_default();
            if path.is_empty() || path == prev_path {
                return false;
            }
            let core_idle = mpv
                .get_property::<bool>("core-idle")
                .unwrap_or(true);
            let stalled = mpv
                .get_property::<bool>("paused-for-cache")
                .unwrap_or(false);
            let seeking = mpv
                .get_property::<bool>("seeking")
                .unwrap_or(false);
            !core_idle && !stalled && !seeking
        } else {
            false
        }
    }

    /// Re-position the backup window on top of the main one. Same coords.
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

impl Drop for BackupPlayer {
    fn drop(&mut self) {
        unsafe {
            (self.xlib.XDestroyWindow)(self.display, self.window);
            (self.xlib.XFlush)(self.display);
        }
    }
}

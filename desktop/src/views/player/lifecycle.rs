//! Channel switching (load_state), memory cache (preload_channel /
//! set_memory_capacity / purge_memory_cache), and playback lifecycle
//! controls (hide_video / show_video / start_switch / stop_playback /
//! force_play). Extracted from `views/player.rs` to keep the main
//! file under the 800-LOC target set by the audit.
//!
//! Child module of `views::player`, so it inherits private-field
//! access to `PlayerView` — no `pub(super)` on struct fields needed.

use super::PlayerView;
#[cfg(target_os = "linux")]
use super::{extract_video_id, log_quality};
#[cfg(target_os = "linux")]
use super::{AutoAdvanced, BackupPlayer, MemorizedChannel, MemoryChanged};
use crate::mpv_try;
use gpui::Context;

impl PlayerView {
    pub fn seek(&self, seconds: f64) {
        {
            let mpv = &self.mpv;
            mpv_try!(mpv.set_property("time-pos", seconds), "main seek", seconds);
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
                // Memory-cache backup is already the visible surface
                // — no transition window to fill. Drop any pending
                // channel-switch snapshot so the next render
                // repositions the backup to the video area (instead
                // of the -10000 off-screen pin armed by
                // show_snapshot). Without this the static thumbnail
                // would sit on top of a live memory-cached video
                // for the 3-s safety timeout.
                self.clear_snapshot();
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
                    {
            let mpv = &self.mpv;
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
                        // Prefer the server-pre-resolved LQ URL when
                        // fresh — lets mpv skip the yt-dlp step
                        // entirely on the backup (~200-400 ms saved on
                        // cold first-frame).
                        let backup_resolved = state
                            .resolved_url_lq
                            .as_deref()
                            .filter(|s| !s.is_empty() && state.resolved_url_is_fresh());
                        b.load(&url, backup_resolved, state.seek_to);
                        self.pending_backup_reveal = true;
                        log_quality(&format!(
                            "channel/video switch → backup loading {} (resolved={}, parallel main load)",
                            state.video_id,
                            backup_resolved.is_some()
                        ));
                    }
                }
            }
            let _ = cx;

            // Main loads IN PARALLEL with backup. The existing swap-up
            // logic will reveal main once it's ready (VIDEO_RECONFIG +
            // 3 s on backup).
            {
            let mpv = &self.mpv;
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
                    mpv_try!(mpv.command("playlist-clear", &[]), "main playlist-clear");
                    // Start muted at volume 0 — the volume fade-in
                    // (in finish_audio_fade_in) ramps to 100 once the
                    // first frame is on screen.
                    mpv_try!(mpv.set_property("volume", 0i64), "main set volume=0");
                    mpv_try!(mpv.set_property("mute", false), "main unmute");
                    mpv_try!(
                        mpv.set_property("start", format!("+{}", state.seek_to)),
                        "main set start",
                        state.seek_to
                    );
                    // Pre-resolved URL path: skip yt-dlp entirely by
                    // handing mpv the final googlevideo URL the
                    // server's url-resolver cached for us. Huge cold-
                    // zap win (~200-800 ms off first-frame). Falls
                    // back to the youtube.com URL + ytdl_hook when
                    // the URL is absent or stale.
                    let main_resolved = state
                        .resolved_url
                        .as_deref()
                        .filter(|s| !s.is_empty() && state.resolved_url_is_fresh());
                    let chosen = main_resolved.unwrap_or(url.as_str());
                    let ytdl_on = main_resolved.is_none();
                    mpv_try!(mpv.set_property("ytdl", ytdl_on), "main set ytdl", ytdl_on);
                    mpv_try!(mpv.command("loadfile", &[chosen]), "main loadfile", chosen);
                    log_quality(&format!(
                        "main loadfile {} (resolved={})",
                        state.video_id, main_resolved.is_some()
                    ));
                }
            }
        }

        // Prefetch the upcoming video, if known and not already queued.
        if let Some(next_id) = state.next_video_id.as_ref() {
            if self.queued_next_id.as_deref() != Some(next_id.as_str())
                && self.current_video_id.as_deref() != Some(next_id.as_str())
            {
                let next_url = format!("https://www.youtube.com/watch?v={}", next_id);
                {
            let mpv = &self.mpv;
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
                    mpv_try!(
                        mpv.command("loadfile", &[&next_url, "append-play"]),
                        "main prefetch loadfile",
                        &next_url
                    );
                }
                self.queued_next_id = Some(next_id.clone());
            }
        }
    }

    /// Tell the backup mpv instance to load the same video at low quality so
    /// it's ready (already decoding, audio muted, window unmapped) when the
    /// main instance stalls.
    #[cfg(target_os = "linux")]
    pub(super) fn attach_backup_quality(&mut self, video_id: &str, _cx: &mut Context<Self>) {
        self.cache_stall_since = None;
        self.using_backup = false;
        self.backup_since = None;

        if let Some(b) = self.backup.as_mut() {
            let url = format!("https://www.youtube.com/watch?v={}", video_id);
            // Make sure the backup is hidden + muted while main plays.
            b.hide();
            // attach_backup_quality runs on auto-advance (new videoId
            // mid-channel). No pre-resolved URL is available here
            // because the server cache is keyed on channelId and the
            // sweep only touches "current" videos — auto-advance
            // happens faster than a sweep cycle. Fall back to
            // ytdl_hook.
            b.load(&url, None, 0.0);
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
        // preload_channel fires from hover → no tv:state round-trip
        // yet, so we haven't seen a resolvedUrl for this channel.
        // Ship ytdl_hook; it's fine because this runs in the background
        // and the user won't watch the result until they click (at
        // which point the freshly cached URL, if any, would be used
        // via the load_state path — but freeze is about parking the
        // demuxer, not the URL).
        backup.load(url, None, seek_to);
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

}

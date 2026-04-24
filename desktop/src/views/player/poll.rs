//! Adaptive poll loop for the main PlayerView entity.
//!
//! The closure body lives here (~400 LOC) so the parent `player.rs`
//! can focus on struct def + `new()` construction orchestration.
//! Extracted verbatim from the old inline `cx.spawn(...)` block in
//! `new()`. Captures:
//!
//!   - `cx.entity().downgrade()` (WeakEntity<PlayerView>) to run
//!     closures inside `.upgrade().update(cx, |p, cx| …)`.
//!   - `Option<Arc<Mutex<Receiver<MenuEvent>>>>` for popup events.
//!
//! Body:
//!   - Pumps popup events via `try_recv` → `PlayerView::handle_menu_event`
//!   - Drains mpv events, detects VIDEO_RECONFIG → marks
//!     `main_first_frame_ready`
//!   - Fires fade-in when main first frame appears (if not using backup)
//!   - Polls backup readiness → reveal + start swap-down timer
//!   - Auto-advance detection via mpv's `path` property change
//!   - Channel-switch deadline / cache-stall / swap-up / swap-down
//!     drift-aware logic
//!   - Adaptive interval: 16 ms during critical phases, 60 ms idle

#![cfg(target_os = "linux")]

use super::PlayerView;
use super::{fade_volume, log_quality, extract_video_id, AutoAdvanced};
use super::{FavoriteToggleFromBadge, MenuEvent, MpvEvent};
use crate::mpv_try;
use gpui::*;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

pub(super) fn start(
    pop_rx: Option<Arc<Mutex<Receiver<MenuEvent>>>>,
    cx: &mut Context<PlayerView>,
) {
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

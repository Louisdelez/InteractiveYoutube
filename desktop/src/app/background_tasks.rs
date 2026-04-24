//! Background setup tasks spawned from `AppView::new()`. Each was an
//! inline `cx.spawn(async move |_, cx| { … }).detach()` block of
//! 15-100 LOC. Extracted here to keep `new()` a thin orchestrator
//! and the parent `app.rs` under the audit's <500-LOC target.
//!
//! Captures are passed as explicit arguments — no more mystery closure
//! state. `WeakEntity<AppView>` handles the standard
//! `.upgrade().update(cx, |app, _| …)` dance.

use super::*;
use crate::services::frame_cache;

/// Fetch a single YouTube thumbnail for a favorite channel's current
/// videoId. Inserts into `AppView::frame_cache` on success. Silent
/// on failure — if the network hiccups or YouTube returns a 404 for
/// this particular video, we just don't have a snapshot for it and
/// the click falls back to the existing "previous frame frozen"
/// behaviour. No retries; the next auto-advance (or favorite
/// re-toggle) will try again.
///
/// Deduplicates via the cache's `needs_refresh` — if the cache
/// already has this exact videoId for this channel, we skip the fetch
/// entirely. Cheap to call from the dispatch hook on every tv:state /
/// tv:sync without flooding img.youtube.com.
pub fn fetch_snapshot(
    entity: WeakEntity<AppView>,
    channel_id: String,
    video_id: String,
    cx: &mut Context<AppView>,
) {
    cx.spawn(async move |_, cx| {
        let (tx, rx) = std::sync::mpsc::channel::<Option<Vec<u8>>>();
        let vid = video_id.clone();
        std::thread::spawn(move || {
            let _ = tx.send(frame_cache::fetch_thumbnail_bytes(&vid));
        });
        // Poll every 100 ms until the blocking fetch returns (typical
        // img.youtube.com fetch is 100-500 ms on a reasonable
        // connection).
        loop {
            match rx.try_recv() {
                Ok(Some(bytes)) if !bytes.is_empty() => {
                    let format = frame_cache::guess_format(&bytes);
                    let image = std::sync::Arc::new(Image::from_bytes(format, bytes));
                    if let Some(e) = entity.upgrade() {
                        let _ = cx.update(|cx| {
                            e.update(cx, |app: &mut AppView, cx| {
                                app.frame_cache.insert(channel_id.clone(), video_id.clone(), image);
                                cx.notify();
                            });
                        });
                    }
                    return;
                }
                Ok(_) => return, // empty / None — give up silently
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }
    })
    .detach();
}

/// Channels fetch + avatar download + badge refresh. ~100 LOC.
pub(super) fn channels_and_avatars(
    sidebar_fetch: Entity<SidebarView>,
    entity_for_avatar: WeakEntity<AppView>,
    cx: &mut Context<AppView>,
) {
            cx.spawn(async move |_, cx| {
                let (tx, rx) = std::sync::mpsc::channel::<
                    Vec<(String, String, String, String)>
                >();
                std::thread::spawn(move || {
                    if let Ok(channels) = api::fetch_channels() {
                        let _ = tx.send(
                            channels.into_iter()
                                .map(|c| (c.id, c.name, c.handle, c.avatar))
                                .collect(),
                        );
                    }
                });
                // Wait for server channels (or give up after ~3s)
                let mut attempts = 30;
                loop {
                    if let Ok(channels) = rx.try_recv() {
                        cx.update(|cx| {
                            sidebar_fetch.update(cx, |s, cx| {
                                s.set_channels_from_server(channels);
                                cx.notify();
                            });
                        });
                        break;
                    }
                    attempts -= 1;
                    if attempts == 0 {
                        break;
                    }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }

                // Now download avatars for all current channels (from hardcoded or server list)
                let to_fetch: Vec<(String, String)> = {
                    let s = sidebar_fetch.clone();
                    cx.update(move |cx| -> Vec<(String, String)> {
                        s.read(cx)
                            .channels
                            .iter()
                            .filter(|c| !c.avatar_url.is_empty())
                            .map(|c| (c.id.clone(), c.avatar_url.clone()))
                            .collect()
                    })
                };

                let (av_tx, av_rx) = std::sync::mpsc::channel::<(String, Vec<u8>)>();
                for (id, url) in to_fetch {
                    let tx = av_tx.clone();
                    std::thread::spawn(move || {
                        if let Ok(bytes) = api::fetch_bytes(&url) {
                            let _ = tx.send((id, bytes));
                        }
                    });
                }
                drop(av_tx);

                loop {
                    match av_rx.try_recv() {
                        Ok((id, bytes)) => {
                            let format = detect_image_format(&bytes);
                            let image = std::sync::Arc::new(Image::from_bytes(format, bytes.clone()));
                            let id_for_app = id.clone();
                            let bytes_for_app = bytes.clone();
                            cx.update(|cx| {
                                sidebar_fetch.update(cx, |s, cx| {
                                    s.set_avatar(id, image);
                                    cx.notify();
                                });
                                if let Some(this) = entity_for_avatar.upgrade() {
                                    this.update(cx, |app: &mut AppView, cx| {
                                        // Stash bytes so the badge can use them
                                        // any time, and refresh the badge if
                                        // this avatar belongs to the active
                                        // channel.
                                        app.avatar_bytes.insert(id_for_app.clone(), bytes_for_app.clone());
                                        if app.current_channel_id.as_deref() == Some(id_for_app.as_str()) {
                                            let name = app
                                                .sidebar
                                                .read(cx)
                                                .channels
                                                .iter()
                                                .find(|c| c.id == id_for_app)
                                                .map(|c| c.name.clone())
                                                .unwrap_or_default();
                                            #[cfg(target_os = "linux")]
                                            {
                                                let is_fav = app.settings.favorites.contains(&id_for_app);
                                                app.player.update(cx, |p, _| {
                                                    p.set_channel_badge(name, bytes_for_app, is_fav);
                                                });
                                            }
                                        }
                                    });
                                }
                            });
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            cx.background_executor().timer(Duration::from_millis(100)).await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            })
            .detach();
}

/// Pre-fetch last_state_per_channel for instant-zap warm cache.
pub(super) fn state_prefetch(
    sidebar_prefetch: Entity<SidebarView>,
    entity_for_prefetch: WeakEntity<AppView>,
    cx: &mut Context<AppView>,
) {
            cx.spawn(async move |_, cx| {
                // Wait for the channel list to populate (avatar spawn
                // polls it too via its own retry; 5 s is the same budget).
                let mut channel_ids: Vec<String> = Vec::new();
                for _ in 0..50 {
                    let got: Vec<String> = cx.update(|cx| {
                        sidebar_prefetch
                            .read(cx)
                            .channels
                            .iter()
                            .map(|c| c.id.clone())
                            .collect::<Vec<_>>()
                    });
                    if !got.is_empty() {
                        channel_ids = got;
                        break;
                    }
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                }
                if channel_ids.is_empty() {
                    return;
                }
                let (tx, rx) = std::sync::mpsc::channel::<(
                    String,
                    crate::models::tv_state::TvState,
                )>();
                // Bounded worker pool (8 threads) pulling ids off a
                // shared queue. Firing all 48 HTTP requests at once
                // briefly spawned ~48 threads + reqwest internals, and
                // each socket hit the server at the same time. 8 × 6
                // sequential requests = the same ~100 ms total with a
                // flat 8-thread footprint.
                let queue = Arc::new(Mutex::new(channel_ids));
                for _ in 0..8 {
                    let tx = tx.clone();
                    let queue = queue.clone();
                    std::thread::spawn(move || loop {
                        let next = { queue.lock().ok().and_then(|mut q| q.pop()) };
                        let Some(id) = next else { break };
                        if let Ok(state) = api::fetch_tv_state(&id) {
                            let _ = tx.send((id, state));
                        }
                    });
                }
                drop(tx);
                loop {
                    match rx.try_recv() {
                        Ok((id, state)) => {
                            if let Some(this) = entity_for_prefetch.upgrade() {
                                let _ = cx.update(|cx| {
                                    this.update(cx, |app: &mut AppView, cx| {
                                        // Same dispatch-path hook : if this
                                        // channel is a favorite and its
                                        // thumbnail cache is stale/missing,
                                        // fetch it now. Handles the boot
                                        // window before the first tv:sync
                                        // arrives (~15 s after connect).
                                        let is_fav = app.settings.favorites.iter().any(|f| f == &id);
                                        let needs = is_fav && app.frame_cache.needs_refresh(&id, &state.video_id);
                                        app.last_state_per_channel.insert(id.clone(), state.clone());
                                        if needs {
                                            fetch_snapshot(
                                                cx.entity().downgrade(),
                                                id,
                                                state.video_id,
                                                cx,
                                            );
                                        }
                                    });
                                });
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            cx.background_executor()
                                .timer(Duration::from_millis(50))
                                .await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            })
            .detach();
}

/// Periodic disk snapshot of the state cache.
pub(super) fn state_cache_save(
    entity_for_save: WeakEntity<AppView>,
    cx: &mut Context<AppView>,
) {
            cx.spawn(async move |_, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(30)).await;
                    let Some(e) = entity_for_save.upgrade() else { break };
                    let map: std::collections::HashMap<
                        String,
                        crate::models::tv_state::TvState,
                    > = cx.update(|cx| e.read(cx).last_state_per_channel.clone());
                    if !map.is_empty() {
                        std::thread::spawn(move || {
                            crate::services::state_cache::save(&map);
                        });
                    }
                }
            })
            .detach();
}

/// /health ping every 2s → WiFi-bars indicator.
pub(super) fn latency_probe(
    entity_for_ping: WeakEntity<AppView>,
    cx: &mut Context<AppView>,
) {
            cx.spawn(async move |_, cx| {
                let (tx, rx) = std::sync::mpsc::channel::<Option<u32>>();
                std::thread::spawn(move || {
                    let client = reqwest::blocking::Client::builder()
                        .timeout(Duration::from_millis(1500))
                        .build()
                        .ok();
                    let url = format!("{}/health", crate::config::server_url());
                    loop {
                        let result = client.as_ref().and_then(|c| {
                            let start = std::time::Instant::now();
                            match c.get(&url).send() {
                                Ok(r) if r.status().is_success() => {
                                    Some(start.elapsed().as_millis() as u32)
                                }
                                _ => None,
                            }
                        });
                        if tx.send(result).is_err() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2000));
                    }
                });
                // Tolerance: don't declare offline on a single failed ping.
                // Need OFFLINE_THRESHOLD consecutive failures (≈ threshold ×
                // ping interval = 3 × 2s = 6s) before stopping playback.
                // Comebacks are immediate (1 success → online) so good signal
                // restores playback fast.
                const OFFLINE_THRESHOLD: u32 = 3;
                let mut consecutive_fails: u32 = 0;
                loop {
                    let mut latest: Option<Option<u32>> = None;
                    while let Ok(v) = rx.try_recv() {
                        latest = Some(v);
                    }
                    if let Some(v) = latest {
                        // Update the failure counter from the latest ping.
                        if v.is_some() {
                            consecutive_fails = 0;
                        } else {
                            consecutive_fails = consecutive_fails.saturating_add(1);
                        }
                        // Connection is "really" offline only after enough
                        // failures in a row.
                        let now_connected = v.is_some() || consecutive_fails < OFFLINE_THRESHOLD;
                        if let Some(e) = entity_for_ping.upgrade() {
                            let _ = cx.update(|cx| {
                                e.update(cx, |this: &mut AppView, cx| {
                                    let was = this.connected;
                                    this.latency_ms = v;
                                    this.connected = now_connected;
                                    // Hide / restore the mpv X11 child window so
                                    // the GPUI offline overlay is actually visible
                                    // (mpv draws above any GPUI element).
                                    if was != this.connected {
                                        if this.connected {
                                            // Server back: show mpv + ask for
                                            // fresh state so the right channel
                                            // resumes immediately (otherwise
                                            // we'd wait up to 15s for tv:sync).
                                            #[cfg(target_os = "linux")]
                                            this.player.update(cx, |p, _| p.show_video());
                                            if let Some(ch) = this.current_channel_id.clone() {
                                                let _ = this.cmd_tx.send(
                                                    ClientCommand::SwitchChannel(ch),
                                                );
                                            } else {
                                                let _ = this.cmd_tx.send(ClientCommand::RequestState);
                                            }
                                        } else {
                                            // Server gone: hard-stop mpv. No
                                            // server = no truth to play.
                                            #[cfg(target_os = "linux")]
                                            this.player.update(cx, |p, _| {
                                                p.stop_playback();
                                                p.hide_video();
                                            });
                                        }
                                    }
                                    let _ = was;
                                    cx.notify();
                                });
                            });
                        }
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                }
            })
            .detach();
}

/// Probe /api/auth/me at startup — restores session if cookie present.
pub(super) fn me_probe(
    entity_for_me: WeakEntity<AppView>,
    cx: &mut Context<AppView>,
) {
            cx.spawn(async move |_, cx| {
                let (tx, rx) = std::sync::mpsc::channel::<Option<User>>();
                std::thread::spawn(move || {
                    let _ = tx.send(api::fetch_me().ok());
                });
                loop {
                    if let Ok(maybe_user) = rx.try_recv() {
                        if let Some(u) = maybe_user {
                            if let Some(e) = entity_for_me.upgrade() {
                                let _ = cx.update(|cx| {
                                    e.update(cx, |this: &mut AppView, cx| {
                                        this.user = Some(u);
                                        this.sidebar.update(cx, |s, cx| {
                                            s.set_logged_in(true);
                                            cx.notify();
                                        });
                                        spawn_pull_user_settings(cx);
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

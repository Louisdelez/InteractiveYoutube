// Sub-modules live in `src/app/`. Extra `impl AppView` blocks in the
// child modules inherit private-field access to AppView.
mod fps;
mod helpers;
mod modals;
mod render;
use fps::FpsCounter;
use helpers::{
    detect_image_format, koala_logo, latency_color, signal_bars,
    spawn_pull_user_settings, spawn_push_user_settings,
};


use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::i18n::t;
use crate::services::api::{self, User};
use crate::services::websocket::{self, ClientCommand, ServerEvent};
use crate::services::settings::{self, Settings};
use crate::views::auth::{AuthEvent, AuthView};
use crate::views::settings_modal::{SettingsEvent, SettingsModal};
use crate::views::chat::{ChatSend, ChatView};
use crate::views::icons::{IconCache, IconName};
use crate::views::player::{AutoAdvanced, FavoriteToggleFromBadge, MemoryChanged, PlayerView};
use crate::views::planning::{PlanningClose, PlanningView};
use crate::views::sidebar::{ChannelFavoriteToggle, ChannelHovered, ChannelSelected, SidebarView};
use crate::views::tooltip::TooltipOverlay;
use std::cell::RefCell;
use std::rc::Rc;

pub struct AppView {
    pub(super) sidebar: Entity<SidebarView>,
    pub(super) player: Entity<PlayerView>,
    pub(super) chat: Entity<ChatView>,
    /// FPS counter: timestamps of recent render() calls, rolling 1 s
    /// window. A 1 s timer in render() re-triggers cx.notify() so the
    /// counter keeps updating during idle — up from the old 200 ms
    /// tick so the bare overhead of "forced re-render for telemetry"
    /// is 5× cheaper.
    pub(super) frame_times: FpsCounter,
    /// (channel_id, channel_name) of the currently hovered sidebar
    /// button. Used by the debounced hover-preload so we can check
    /// "still hovering the same channel 300 ms later?" before kicking
    /// off the mpv warm-up.
    pub(super) hovered_channel: Option<(String, String)>,
    /// Monotonic version counter bumped on every hover change. The
    /// preload closure captures the version at schedule time and
    /// compares to the current value before firing — if the hover
    /// changed in the meantime, the preload is skipped.
    pub(super) hover_preload_version: std::cell::Cell<u64>,
    pub(super) current_channel_id: Option<String>,
    /// Channel id the client has asked the server to switch to, but whose
    /// confirming `tv:state` hasn't arrived yet. While this is `Some(id)`,
    /// any `tv:state` / `tv:sync` for a DIFFERENT channel is dropped —
    /// it's the stale initial-state the server emits on every socket
    /// (re)connect for its randomly-chosen default room, which would
    /// otherwise yank us off our channel for a fraction of a second
    /// before our SwitchChannel round-trips.
    pub(super) pending_channel_switch: Option<String>,
    /// Raw avatar bytes per channel id, kept around so the player's
    /// "now playing" badge can blit them without re-fetching.
    pub(super) avatar_bytes: std::collections::HashMap<String, Vec<u8>>,
    /// Last accepted tv:state per channel_id. Used for the optimistic
    /// instant-zap: when the user clicks a channel they've recently
    /// visited, we synthesise a rebased TvState from this cache
    /// (seek_to advanced by elapsed wall-clock) and call `load_state`
    /// synchronously — the cached backup mpv is revealed *before* the
    /// server round-trip completes. The real tv:state arriving ~50-
    /// 200 ms later re-enters load_state idempotently and applies any
    /// drift correction.
    pub(super) last_state_per_channel: std::collections::HashMap<String, crate::models::tv_state::TvState>,
    /// Connection status — derived from `latency_ms`. Kept for the player
    /// overlay logic ("server unavailable" curtain).
    pub(super) connected: bool,
    pub(super) maintenance: bool,
    pub(super) maintenance_warning: bool,
    /// Round-trip latency to the server's `/health` endpoint, in milliseconds.
    /// `None` means the last probe failed → server is considered offline.
    pub(super) latency_ms: Option<u32>,
    /// Logged-in user (None = anonymous).
    pub(super) user: Option<User>,
    /// Auth panel visible (replaces the chat panel) when set.
    pub(super) auth: Option<Entity<AuthView>>,
    /// Whether the chat sidebar is visible. Toggled from the topbar.
    pub(super) chat_open: bool,
    /// Total viewers across all channels — pushed by the server's
    /// `viewers:total` event, displayed in the topbar.
    pub(super) total_viewers: usize,
    /// Settings modal (gear icon in topbar). None = closed.
    pub(super) settings_modal: Option<Entity<SettingsModal>>,
    /// Full-screen planning (TV guide) view. None = closed.
    pub(super) planning: Option<Entity<PlanningView>>,
    /// Close-event subscription for the current planning entity. Replaced
    /// (dropping the old) every time planning reopens — without this, each
    /// open/close cycle leaked a Subscription into `_subscriptions`.
    pub(super) planning_sub: Option<Subscription>,
    /// Persistent user preferences (memory cache size, favourites).
    pub(super) settings: Settings,
    pub(super) search_state: Entity<InputState>,
    pub(super) icons: Rc<RefCell<IconCache>>,
    #[allow(dead_code)]
    pub(super) tooltip: Rc<RefCell<Option<TooltipOverlay>>>,
    #[allow(dead_code)]
    pub(super) cmd_tx: std::sync::mpsc::Sender<ClientCommand>,
    #[allow(dead_code)]
    pub(super) _subscriptions: Vec<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut App) -> Entity<Self> {
        let sidebar = cx.new(|_| SidebarView::new());
        let player = PlayerView::new(window, cx);
        let chat = cx.new(|cx| ChatView::new(window, cx));

        // Start background Socket.IO client
        let (event_tx, event_rx) = mpsc::channel::<ServerEvent>();
        let cmd_tx = websocket::start(event_tx);

        cx.new(|cx| {
            // Search input in topbar
            let search_state = cx.new(|cx| {
                InputState::new(window, cx).placeholder("Rechercher une chaîne…")
            });
            let sidebar_search = sidebar.clone();
            let search_handle = search_state.clone();
            let sub_search = cx.subscribe_in(
                &search_state,
                window,
                move |_this: &mut AppView, _state, _ev: &InputEvent, _window, cx| {
                    let q = search_handle.read(cx).value().to_string();
                    sidebar_search.update(cx, |s, cx| {
                        s.set_search_query(q);
                        cx.notify();
                    });
                },
            );

            // Wire sidebar click → ask the server to switch channel. The
            // server replies with tv:state which is what actually triggers
            // playback. While offline the click does nothing (overlay blocks
            // interaction anyway).
            let cmd_tx_clone = cmd_tx.clone();
            let sub_channel = cx.subscribe(
                &sidebar,
                move |this: &mut AppView, _sidebar, event: &ChannelSelected, cx| {
                    if !this.connected {
                        return;
                    }
                    this.current_channel_id = Some(event.channel_id.clone());
                    // Also arm the pending-switch guard so any stale
                    // tv:state arriving while the server processes our
                    // request is dropped.
                    this.pending_channel_switch = Some(event.channel_id.clone());
                    // Push the new channel's name + avatar to the
                    // X11 "now playing" badge.
                    #[cfg(target_os = "linux")]
                    {
                        let name = event.channel_name.clone();
                        let is_fav = this.settings.favorites.contains(&event.channel_id);
                        if let Some(bytes) = this.avatar_bytes.get(&event.channel_id).cloned() {
                            this.player.update(cx, |p, _| {
                                p.set_channel_badge(name, bytes, is_fav);
                            });
                        }
                    }
                    // Clear local chat immediately so messages from the
                    // previous channel disappear during the switch.
                    this.chat.update(cx, |c, cx| {
                        c.replace_messages(Vec::new(), cx);
                        cx.notify();
                    });
                    // Optimistic instant switch: if we have a recent
                    // tv:state cached for the target channel, rebase
                    // seek_to by elapsed wall-clock and call load_state
                    // synchronously — the backup mpv for this channel
                    // (if present in memory_cache) is revealed *now*,
                    // before the socket.io round-trip completes. The
                    // real server state, arriving ~50-200 ms later,
                    // re-enters load_state idempotently: same video_id
                    // skips the main loadfile, drift correction (4 s
                    // threshold) absorbs any mismatch from our local
                    // rebase. Cold channels (no cached state) fall back
                    // to the normal server-driven path.
                    if let Some(cached) = this
                        .last_state_per_channel
                        .get(&event.channel_id)
                        .cloned()
                    {
                        let now_secs = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(cached.server_time);
                        let elapsed = now_secs.saturating_sub(cached.server_time) as f64;
                        let rebased_seek = cached.seek_to + elapsed;
                        // If the cached video would already have ended,
                        // don't rebase — use the cached state as-is and
                        // let the server tell us the new video. The
                        // backup mpv (paused on the old frame) is still
                        // revealed for a visible instant response.
                        let rebased = if rebased_seek < cached.duration {
                            crate::models::tv_state::TvState {
                                seek_to: rebased_seek,
                                ..cached
                            }
                        } else {
                            cached
                        };
                        this.player.update(cx, |p, cx| p.load_state(&rebased, cx));
                    }
                    let _ = cmd_tx_clone.send(ClientCommand::SwitchChannel(event.channel_id.clone()));
                    // Ask the server for the new channel's chat history.
                    let _ = cmd_tx_clone.send(ClientCommand::ChatChannelChanged(event.channel_id.clone()));
                },
            );

            // Wire mpv auto-advance → ask server for fresh state so any drift
            // gets corrected immediately (no need to wait for the 15 s tv:sync).
            let cmd_tx_advance = cmd_tx.clone();
            let sub_advance = cx.subscribe(
                &player,
                move |_this: &mut AppView, _player, _ev: &AutoAdvanced, _cx| {
                    let _ = cmd_tx_advance.send(ClientCommand::RequestState);
                },
            );

            // Star button on the channel badge → toggle favourite for
            // the currently-active channel.
            let sub_badge_fav = cx.subscribe(
                &player,
                move |this: &mut AppView, _player, _ev: &FavoriteToggleFromBadge, cx| {
                    let Some(id) = this.current_channel_id.clone() else { return };
                    if let Some(pos) = this.settings.favorites.iter().position(|x| x == &id) {
                        this.settings.favorites.remove(pos);
                    } else {
                        this.settings.favorites.push(id.clone());
                    }
                    settings::save(&this.settings);
                    spawn_push_user_settings(this.settings.clone());
                    let favs = this.settings.favorites.clone();
                    let is_fav = favs.contains(&id);
                    this.sidebar.update(cx, |s, cx| {
                        s.set_favorites(favs);
                        cx.notify();
                    });
                    // Refresh the badge so the star icon flips state.
                    let name = this
                        .sidebar
                        .read(cx)
                        .channels
                        .iter()
                        .find(|c| c.id == id)
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    let bytes = this.avatar_bytes.get(&id).cloned().unwrap_or_default();
                    #[cfg(target_os = "linux")]
                    this.player.update(cx, |p, _| {
                        p.set_channel_badge(name, bytes, is_fav);
                    });
                },
            );

            // Push the player's memory cache contents to the sidebar so
            // it can render the "Mémoire" section.
            let sidebar_mem = sidebar.clone();
            let sub_memory = cx.subscribe(
                &player,
                move |_this: &mut AppView, _player, ev: &MemoryChanged, cx| {
                    let ids = ev.0.clone();
                    sidebar_mem.update(cx, |s, cx| {
                        s.set_memory_channel_ids(ids);
                        cx.notify();
                    });
                },
            );

            // Create X11 tooltip overlay (top-level window above mpv)
            let tooltip = Rc::new(RefCell::new(TooltipOverlay::new()));

            // Counter for debouncing: increments on each event. Delayed-hide
            // tasks check it still matches before hiding (so a fast re-hover
            // cancels the pending hide).
            let hide_version = Rc::new(std::cell::Cell::new(0u64));

            // Right-click on a channel toggles favourite. Persist
            // immediately to local settings (server sync TBD when
            // logged in).
            let sub_favorite = cx.subscribe(
                &sidebar,
                move |this: &mut AppView, _sidebar, event: &ChannelFavoriteToggle, cx| {
                    let id = event.0.clone();
                    if let Some(pos) = this.settings.favorites.iter().position(|x| x == &id) {
                        this.settings.favorites.remove(pos);
                    } else {
                        this.settings.favorites.push(id);
                    }
                    settings::save(&this.settings);
                    spawn_push_user_settings(this.settings.clone());
                    let favs = this.settings.favorites.clone();
                    this.sidebar.update(cx, |s, cx| {
                        s.set_favorites(favs);
                        cx.notify();
                    });
                },
            );

            let tooltip_handle = tooltip.clone();
            let hide_version_handle = hide_version.clone();
            let sub_hover = cx.subscribe(
                &sidebar,
                move |this: &mut AppView, _sidebar, event: &ChannelHovered, cx| {
                    this.hovered_channel = event.0.clone();
                    let v = hide_version_handle.get().wrapping_add(1);
                    hide_version_handle.set(v);
                    // Independent version for the preload debounce — a
                    // rapid series of hovers shouldn't leave multiple
                    // queued preloads; each new hover invalidates the
                    // previous scheduled one.
                    let pv = this.hover_preload_version.get().wrapping_add(1);
                    this.hover_preload_version.set(pv);

                    match &event.0 {
                        Some((id, name)) => {
                            if let Some(tt) = tooltip_handle.borrow_mut().as_mut() {
                                if let Some((mx, my)) = tt.query_pointer() {
                                    tt.show(name, mx + 14, my + 16);
                                }
                            }
                            // Schedule a debounced preload: after 300 ms
                            // of continuous hover on the same channel
                            // (the user has committed to looking at it)
                            // create a parked BackupPlayer for it so
                            // the click is an instant XMapRaised. If
                            // the hover changes before the timer fires,
                            // `hover_preload_version` mismatches and
                            // we bail out — no wasted mpv instance.
                            #[cfg(target_os = "linux")]
                            {
                                let expected_pv = pv;
                                let channel_id = id.clone();
                                cx.spawn(async move |this, cx| {
                                    cx.background_executor()
                                        .timer(Duration::from_millis(300))
                                        .await;
                                    if let Some(e) = this.upgrade() {
                                        let _ = cx.update(|cx| {
                                            e.update(cx, |app: &mut AppView, cx| {
                                                // Hover changed → abort.
                                                if app.hover_preload_version.get() != expected_pv {
                                                    return;
                                                }
                                                // Need a cached state to
                                                // know WHAT URL + seek
                                                // to pre-load. Without
                                                // it we'd have to RTT
                                                // the server first,
                                                // which defeats the
                                                // purpose.
                                                let Some(state) = app
                                                    .last_state_per_channel
                                                    .get(&channel_id)
                                                    .cloned()
                                                else {
                                                    return;
                                                };
                                                let now_secs = std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .map(|d| d.as_secs())
                                                    .unwrap_or(state.server_time);
                                                let elapsed = now_secs
                                                    .saturating_sub(state.server_time)
                                                    as f64;
                                                let seek = (state.seek_to + elapsed)
                                                    .min(state.duration.max(0.0));
                                                let url = format!(
                                                    "https://www.youtube.com/watch?v={}",
                                                    state.video_id
                                                );
                                                app.player.update(cx, |p, cx| {
                                                    p.preload_channel(&channel_id, &url, seek, cx);
                                                });
                                            });
                                        });
                                    }
                                })
                                .detach();
                            }
                        }
                        None => {
                            // Delayed hide: only hide if no other hover fires in 80ms
                            let tt = tooltip_handle.clone();
                            let version = hide_version_handle.clone();
                            let expected = v;
                            cx.spawn(async move |_, cx| {
                                cx.background_executor().timer(Duration::from_millis(60)).await;
                                if version.get() == expected {
                                    if let Some(tt) = tt.borrow_mut().as_mut() {
                                        tt.hide();
                                    }
                                }
                            })
                            .detach();
                        }
                    }
                    cx.notify();
                },
            );

            // Wire chat Enter → send via WebSocket. The server broadcasts the
            // message back to all clients (including us), so we do NOT echo it
            // locally — that would show the message twice (once as "Moi", once
            // as the server-reported username).
            let cmd_tx_chat = cmd_tx.clone();
            let sub_chat = cx.subscribe(
                &chat,
                move |_this: &mut AppView, _chat, event: &ChatSend, _cx| {
                    let _ = cmd_tx_chat.send(ClientCommand::SendChat(event.text.clone()));
                },
            );

            // Fetch channel list from server in background, then download avatars
            let sidebar_fetch = sidebar.clone();
            let entity_for_avatar = cx.entity().downgrade();
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

            // Prefetch tv:state for every channel so the very first click
            // on a channel that was never visited also hits the optimistic
            // instant-zap path (the click handler synthesises a rebased
            // state from `last_state_per_channel` and calls load_state
            // synchronously before the socket round-trip completes). We
            // fire one HTTP request per channel in parallel — the server
            // handles each with a single playlist lookup, total ~100 ms
            // for 48 chaînes. Cost: ~60 kB of JSON; benefit: every click
            // henceforth is instant regardless of visit history.
            let sidebar_prefetch = sidebar.clone();
            let entity_for_prefetch = cx.entity().downgrade();
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
                                    this.update(cx, |app: &mut AppView, _| {
                                        app.last_state_per_channel.insert(id, state);
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

            // Periodically snapshot `last_state_per_channel` to disk so
            // a fresh boot of the app starts with a warm cache. Live
            // tv:state / tv:sync continue to overwrite entries in
            // memory throughout the session; we just push the current
            // view to disk every 30 s. 30 s is long enough to avoid
            // disk thrashing (tv:sync fires every 15 s per channel) and
            // short enough that a crash loses at most half a minute of
            // staleness vs. what we could have persisted.
            let entity_for_save = cx.entity().downgrade();
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

            // Poll the server-event channel from GPUI's executor and dispatch to views
            let rx = Arc::new(Mutex::new(event_rx));
            let player_poll = player.clone();
            let chat_poll = chat.clone();
            let entity_for_status = cx.entity().downgrade();
            let cmd_tx_for_pseudo = cmd_tx.clone();
            cx.spawn(async move |_this, cx| {
                loop {
                    // Drain all pending events
                    let events: Vec<ServerEvent> = {
                        if let Ok(rx) = rx.lock() {
                            std::iter::from_fn(|| rx.try_recv().ok()).collect()
                        } else {
                            Vec::new()
                        }
                    };

                    if !events.is_empty() {
                        let player_u = player_poll.clone();
                        let chat_u = chat_poll.clone();
                        cx.update(|cx| {
                            for ev in events {
                                match ev {
                                    ServerEvent::TvState(state) | ServerEvent::TvSync(state) => {
                                        // Strict policy: the server NEVER gets to change
                                        // our chaîne on its own. A state for a DIFFERENT
                                        // channel than what we're watching is accepted
                                        // ONLY if the user (or a reconnect re-assert)
                                        // explicitly asked for that channel via
                                        // `pending_channel_switch`. Anything else — the
                                        // server's initial random-room emit, a tv:sync
                                        // delivered to our socket while the server still
                                        // thinks we're in a different room, a nodemon
                                        // restart dropping us in Amixem — is silently
                                        // ignored. Updates for the SAME channel always
                                        // pass through (they carry drift corrections
                                        // and auto-advance to the next video in the
                                        // playlist).
                                        let mut accept = true;
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, _cx| {
                                                let same_channel = this
                                                    .current_channel_id
                                                    .as_deref()
                                                    == Some(state.channel_id.as_str());
                                                let user_asked_for = this
                                                    .pending_channel_switch
                                                    .as_deref()
                                                    == Some(state.channel_id.as_str());
                                                if same_channel {
                                                    // drift / auto-advance — accept.
                                                    // Also clear pending if it matches
                                                    // (reconnect re-assert confirmed).
                                                    if user_asked_for {
                                                        this.pending_channel_switch = None;
                                                    }
                                                } else if user_asked_for {
                                                    // confirming our SwitchChannel
                                                    this.pending_channel_switch = None;
                                                } else if this.current_channel_id.is_none() {
                                                    // First-ever state: anchor here
                                                    // so subsequent stale states are
                                                    // rejected.
                                                    this.current_channel_id =
                                                        Some(state.channel_id.clone());
                                                } else {
                                                    accept = false;
                                                }
                                            });
                                        }
                                        if accept {
                                            // Remember the latest accepted state
                                            // per channel for the optimistic
                                            // instant-zap at click time.
                                            if let Some(e) = entity_for_status.upgrade() {
                                                e.update(cx, |this, _cx| {
                                                    this.last_state_per_channel
                                                        .insert(state.channel_id.clone(), state.clone());
                                                });
                                            }
                                        }
                                        if !accept {
                                            if let Some(e) = entity_for_status.upgrade() {
                                                e.update(cx, |this, _cx| {
                                                    if let Some(ref ch) = this.current_channel_id {
                                                        this.pending_channel_switch = Some(ch.clone());
                                                        let _ = cmd_tx_for_pseudo
                                                            .send(ClientCommand::SwitchChannel(ch.clone()));
                                                    }
                                                });
                                            }
                                            continue;
                                        }
                                        player_u.update(cx, |p, cx| p.load_state(&state, cx));
                                    }
                                    ServerEvent::ChatMessage { username, text, color, time } => {
                                        chat_u.update(cx, |c, cx| {
                                            c.push_message(username, text, color, time, cx);
                                            cx.notify();
                                        });
                                    }
                                    ServerEvent::ChatHistory(messages) => {
                                        chat_u.update(cx, |c, cx| {
                                            c.replace_messages(messages, cx);
                                            cx.notify();
                                        });
                                    }
                                    ServerEvent::ChatCleared => {
                                        chat_u.update(cx, |c, cx| {
                                            c.clear_messages(cx);
                                            cx.notify();
                                        });
                                    }
                                    ServerEvent::ViewerCount { count } => {
                                        chat_u.update(cx, |c, cx| {
                                            c.set_viewer_count(count);
                                            cx.notify();
                                        });
                                    }
                                    ServerEvent::ViewerTotal { total } => {
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, cx| {
                                                this.total_viewers = total;
                                                cx.notify();
                                            });
                                        }
                                    }
                                    ServerEvent::MaintenanceWarning => {
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, cx| {
                                                this.maintenance_warning = true;
                                                cx.notify();
                                            });
                                        }
                                    }
                                    ServerEvent::MaintenanceStart => {
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, cx| {
                                                this.maintenance_warning = false;
                                                this.maintenance = true;
                                                cx.notify();
                                            });
                                        }
                                    }
                                    ServerEvent::MaintenanceEnd => {
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, cx| {
                                                this.maintenance = false;
                                                this.maintenance_warning = false;
                                                cx.notify();
                                            });
                                        }
                                    }
                                    ServerEvent::PlaylistUpdated { channel_id } => {
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, cx| {
                                                if let Some(ref planning) = this.planning {
                                                    planning.update(cx, |p, cx| {
                                                        p.on_playlist_updated(&channel_id, cx);
                                                    });
                                                }
                                            });
                                        }
                                    }
                                    ServerEvent::Connected => {
                                        // Snapshot the remembered channel
                                        // BEFORE flipping `connected = true`
                                        // (done inside the update closure).
                                        // The server picks a RANDOM default
                                        // channel on every socket connect,
                                        // so after a brief disconnect (WiFi
                                        // hiccup, server restart) it would
                                        // yank the user off whatever they
                                        // were watching — often landing on
                                        // the first chaîne (Amixem) or any
                                        // other by lottery. We immediately
                                        // re-assert our current channel
                                        // here; the next tv:state arrives
                                        // for our chaîne, overriding the
                                        // server's random pick.
                                        let mut remembered_channel: Option<String> = None;
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, cx| {
                                                this.connected = true;
                                                remembered_channel = this.current_channel_id.clone();
                                                this.pending_channel_switch = remembered_channel.clone();
                                                cx.notify();
                                            });
                                        }
                                        if let Some(ch) = remembered_channel {
                                            let _ = cmd_tx_for_pseudo
                                                .send(ClientCommand::SwitchChannel(ch.clone()));
                                            let _ = cmd_tx_for_pseudo
                                                .send(ClientCommand::ChatChannelChanged(ch));
                                        }
                                        // Push the per-session anonymous
                                        // pseudo + colour to the server.
                                        // Logged-in users override server-
                                        // side, so this is harmless if the
                                        // user is authenticated.
                                        let _ = cmd_tx_for_pseudo.send(
                                            ClientCommand::SetAnonymousName {
                                                name: crate::services::pseudo::get_or_create_pseudo(),
                                                color: crate::services::pseudo::get_or_create_color(),
                                            },
                                        );
                                    }
                                    ServerEvent::Disconnected => {
                                        if let Some(e) = entity_for_status.upgrade() {
                                            e.update(cx, |this, cx| {
                                                this.connected = false;
                                                cx.notify();
                                            });
                                        }
                                    }
                                }
                            }
                        });
                    }

                    cx.background_executor().timer(Duration::from_millis(33)).await;
                }
            })
            .detach();

            // Ping the server's /health endpoint every 2s to measure latency.
            // The result drives the WiFi-bars indicator + offline overlay.
            let entity_for_ping = cx.entity().downgrade();
            cx.spawn(async move |_, cx| {
                let (tx, rx) = std::sync::mpsc::channel::<Option<u32>>();
                std::thread::spawn(move || {
                    let client = reqwest::blocking::Client::builder()
                        .timeout(Duration::from_millis(1500))
                        .build()
                        .ok();
                    let url = format!("{}/health", crate::config::SERVER_URL);
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

            // Probe /api/auth/me at startup — if a session cookie exists from
            // a previous run we'll be auto-logged-in.
            let entity_for_me = cx.entity().downgrade();
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

            // Load persistent user settings, then push them into the
            // player (memory cache size + preferred quality) and
            // sidebar (favourites).
            let initial_settings = settings::load();
            #[cfg(target_os = "linux")]
            player.update(cx, |p, _| {
                p.set_memory_capacity(initial_settings.memory_capacity);
            });
            if initial_settings.preferred_quality != 0 {
                let q = initial_settings.preferred_quality as usize;
                player.update(cx, |p, cx| p.set_quality(q, cx));
            }
            sidebar.update(cx, |s, cx| {
                s.set_favorites(initial_settings.favorites.clone());
                cx.notify();
            });

            // Tell the server which channel we want; the player will start
            // playing only when the server pushes back tv:state. We do NOT
            // call player.navigate() here — the server is the source of
            // truth, so without a server response there is no playback.
            if let Some((id, name, _handle)) = sidebar.read(cx).selected_channel() {
                let _ = cmd_tx.send(ClientCommand::SwitchChannel(id.clone()));
                #[cfg(target_os = "linux")]
                {
                    let is_fav = initial_settings.favorites.contains(&id);
                    player.update(cx, |p, _| {
                        p.set_channel_badge(name, Vec::new(), is_fav);
                    });
                }
            }
            let initial_channel_id = sidebar.read(cx).selected_channel().map(|c| c.0);

            Self {
                sidebar,
                player,
                chat,
                frame_times: FpsCounter::new(),
                hovered_channel: None,
                hover_preload_version: std::cell::Cell::new(0),
                current_channel_id: initial_channel_id,
                pending_channel_switch: None,
                avatar_bytes: std::collections::HashMap::new(),
                // Bootstrap from the last session's snapshot. Stale
                // values get overwritten by the HTTP prefetch + live
                // tv:state / tv:sync stream moments later; having them
                // present at startup means the very first click before
                // prefetch completes (~100-500 ms window) still takes
                // the optimistic instant-zap path instead of waiting
                // for the server round-trip.
                last_state_per_channel: crate::services::state_cache::load(),
                connected: false,
                maintenance: false,
                maintenance_warning: false,
                latency_ms: None,
                user: None,
                auth: None,
                chat_open: true,
                total_viewers: 0,
                settings_modal: None,
                planning: None,
                planning_sub: None,
                settings: initial_settings,
                tooltip,
                cmd_tx,
                search_state,
                icons: Rc::new(RefCell::new(IconCache::new())),
                _subscriptions: vec![
                    sub_channel,
                    sub_chat,
                    sub_hover,
                    sub_search,
                    sub_advance,
                    sub_memory,
                    sub_favorite,
                    sub_badge_fav,
                ],
            }
        })
    }
}



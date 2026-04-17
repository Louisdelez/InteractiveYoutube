use gpui::*;
use gpui_component::input::{Input, InputEvent, InputState};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    sidebar: Entity<SidebarView>,
    player: Entity<PlayerView>,
    chat: Entity<ChatView>,
    hovered_channel: Option<String>,
    current_channel_id: Option<String>,
    /// Channel id the client has asked the server to switch to, but whose
    /// confirming `tv:state` hasn't arrived yet. While this is `Some(id)`,
    /// any `tv:state` / `tv:sync` for a DIFFERENT channel is dropped —
    /// it's the stale initial-state the server emits on every socket
    /// (re)connect for its randomly-chosen default room, which would
    /// otherwise yank us off our channel for a fraction of a second
    /// before our SwitchChannel round-trips.
    pending_channel_switch: Option<String>,
    /// Raw avatar bytes per channel id, kept around so the player's
    /// "now playing" badge can blit them without re-fetching.
    avatar_bytes: std::collections::HashMap<String, Vec<u8>>,
    /// Connection status — derived from `latency_ms`. Kept for the player
    /// overlay logic ("server unavailable" curtain).
    connected: bool,
    /// Round-trip latency to the server's `/health` endpoint, in milliseconds.
    /// `None` means the last probe failed → server is considered offline.
    latency_ms: Option<u32>,
    /// Timestamps of recent render() calls — used to compute an approximate
    /// FPS counter in the topbar. Rolling 1-second window.
    frame_times: Rc<RefCell<std::collections::VecDeque<std::time::Instant>>>,
    /// Logged-in user (None = anonymous).
    user: Option<User>,
    /// Auth panel visible (replaces the chat panel) when set.
    auth: Option<Entity<AuthView>>,
    /// Whether the chat sidebar is visible. Toggled from the topbar.
    chat_open: bool,
    /// Total viewers across all channels — pushed by the server's
    /// `viewers:total` event, displayed in the topbar.
    total_viewers: usize,
    /// Settings modal (gear icon in topbar). None = closed.
    settings_modal: Option<Entity<SettingsModal>>,
    /// Full-screen planning (TV guide) view. None = closed.
    planning: Option<Entity<PlanningView>>,
    /// Close-event subscription for the current planning entity. Replaced
    /// (dropping the old) every time planning reopens — without this, each
    /// open/close cycle leaked a Subscription into `_subscriptions`.
    planning_sub: Option<Subscription>,
    /// Persistent user preferences (memory cache size, favourites).
    settings: Settings,
    search_state: Entity<InputState>,
    icons: Rc<RefCell<IconCache>>,
    #[allow(dead_code)]
    tooltip: Rc<RefCell<Option<TooltipOverlay>>>,
    #[allow(dead_code)]
    cmd_tx: std::sync::mpsc::Sender<ClientCommand>,
    #[allow(dead_code)]
    _subscriptions: Vec<Subscription>,
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
                        c.replace_messages(Vec::new());
                        cx.notify();
                    });
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

                    match &event.0 {
                        Some(name) => {
                            if let Some(tt) = tooltip_handle.borrow_mut().as_mut() {
                                if let Some((mx, my)) = tt.query_pointer() {
                                    tt.show(name, mx + 14, my + 16);
                                }
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
                                                    // drift / auto-advance — accept
                                                } else if user_asked_for {
                                                    // confirming our SwitchChannel
                                                    this.pending_channel_switch = None;
                                                } else if this.current_channel_id.is_none() {
                                                    // First-ever state: no previous
                                                    // chaîne to preserve — accept and
                                                    // anchor here.
                                                } else {
                                                    // Unsolicited channel change
                                                    // attempted by the server. Drop it
                                                    // and re-assert our real chaîne so
                                                    // the server brings us back.
                                                    accept = false;
                                                }
                                            });
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
                                            c.push_message(username, text, color, time);
                                            cx.notify();
                                        });
                                    }
                                    ServerEvent::ChatHistory(messages) => {
                                        chat_u.update(cx, |c, cx| {
                                            c.replace_messages(messages);
                                            cx.notify();
                                        });
                                    }
                                    ServerEvent::ChatCleared => {
                                        chat_u.update(cx, |c, cx| {
                                            c.replace_messages(Vec::new());
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
                                                .send(ClientCommand::SwitchChannel(ch));
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
            // player (memory cache size) and sidebar (favourites).
            let initial_settings = settings::load();
            #[cfg(target_os = "linux")]
            player.update(cx, |p, _| {
                p.set_memory_capacity(initial_settings.memory_capacity);
            });
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
                hovered_channel: None,
                current_channel_id: initial_channel_id,
                pending_channel_switch: None,
                avatar_bytes: std::collections::HashMap::new(),
                connected: false,
                latency_ms: None,
                frame_times: Rc::new(RefCell::new(std::collections::VecDeque::with_capacity(128))),
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

/// Color for the WiFi-bars indicator + the ms label, based on round-trip
/// latency. None (= offline) is rendered red.
fn latency_color(ms: Option<u32>) -> u32 {
    match ms {
        None => 0xef4444,           // red — offline
        Some(v) if v <= 80 => 0x10b981,   // green — excellent
        Some(v) if v <= 200 => 0xeab308,  // yellow — okay
        _ => 0xef4444,                    // red — bad
    }
}

/// 4-bar WiFi-style signal indicator. Bars fill in based on latency:
///   <= 60ms → 4 bars, <= 150ms → 3, <= 300ms → 2, <= 800ms → 1, else 0.
fn signal_bars(latency: Option<u32>) -> impl IntoElement {
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
fn spawn_pull_user_settings(cx: &mut Context<AppView>) {
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
fn spawn_push_user_settings(settings: Settings) {
    std::thread::spawn(move || {
        let _ = api::put_user_settings(&settings);
    });
}

/// App logo (koala) — embedded at compile time, decoded once.
const LOGO_PNG: &[u8] = include_bytes!("../assets/koala-tv.png");
fn koala_logo() -> std::sync::Arc<Image> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<std::sync::Arc<Image>> = OnceLock::new();
    CACHE
        .get_or_init(|| std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, LOGO_PNG.to_vec())))
        .clone()
}

fn detect_image_format(bytes: &[u8]) -> ImageFormat {
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

impl AppView {
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings_modal.is_some() {
            return;
        }
        let initial = self.settings.clone();
        let modal = cx.new(|_| SettingsModal::new(initial));
        let player_clone = self.player.clone();
        let _sub = cx.subscribe(
            &modal,
            move |this: &mut AppView, _modal, ev: &SettingsEvent, cx| {
                match ev.clone() {
                    SettingsEvent::MemoryCapacity(cap) => {
                        this.settings.memory_capacity = cap;
                        settings::save(&this.settings);
                        spawn_push_user_settings(this.settings.clone());
                        #[cfg(target_os = "linux")]
                        player_clone.update(cx, |p, _| p.set_memory_capacity(cap));
                    }
                    SettingsEvent::PurgeMemory => {
                        #[cfg(target_os = "linux")]
                        player_clone.update(cx, |p, _| p.purge_memory_cache());
                    }
                    SettingsEvent::Close => {
                        // Restore mpv before dropping the modal.
                        #[cfg(target_os = "linux")]
                        player_clone.update(cx, |p, _| p.show_video());
                        this.settings_modal = None;
                    }
                }
                cx.notify();
            },
        );
        self._subscriptions.push(_sub);
        // Hide mpv X11 child so the modal isn't covered by it.
        #[cfg(target_os = "linux")]
        self.player.update(cx, |p, _| p.hide_video());
        self.settings_modal = Some(modal);
        cx.notify();
    }

    fn open_planning(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.planning.is_some() {
            return;
        }
        // Build from the current sidebar-known channel list so the picker
        // has every channel, not just whatever hardcoded fallback knew.
        let channels = self
            .sidebar
            .read(cx)
            .channels
            .iter()
            .map(|c| api::ServerChannel {
                id: c.id.clone(),
                name: c.name.clone(),
                handle: c.handle.clone(),
                avatar: c.avatar_url.clone(),
            })
            .collect::<Vec<_>>();
        let initial = self
            .current_channel_id
            .clone()
            .unwrap_or_else(|| channels.first().map(|c| c.id.clone()).unwrap_or_default());
        let planning = cx.new(|cx| PlanningView::new(channels, initial, window, cx));
        let player_clone = self.player.clone();
        let sub = cx.subscribe(
            &planning,
            move |this: &mut AppView, _planning, _ev: &PlanningClose, cx| {
                #[cfg(target_os = "linux")]
                player_clone.update(cx, |p, _| p.show_video());
                this.planning = None;
                this.planning_sub = None;
                cx.notify();
            },
        );
        self.planning_sub = Some(sub);
        // Hide mpv X11 child so the planning grid isn't covered by it.
        #[cfg(target_os = "linux")]
        self.player.update(cx, |p, _| p.hide_video());
        self.planning = Some(planning);
        cx.notify();
    }

    fn open_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let auth = cx.new(|cx| AuthView::new(window, cx));
        let player_clone = self.player.clone();
        let _sub = cx.subscribe(
            &auth,
            move |this: &mut AppView, _auth, ev: &AuthEvent, cx| {
                match ev.clone() {
                    AuthEvent::Authenticated(u) => {
                        this.user = Some(u);
                        this.auth = None;
                        this.sidebar.update(cx, |s, cx| {
                            s.set_logged_in(true);
                            cx.notify();
                        });
                        // Pull the user's saved settings from the
                        // server (overrides local prefs). Background
                        // thread + entity weak ref so we don't block.
                        spawn_pull_user_settings(cx);
                    }
                    AuthEvent::Cancelled => {
                        this.auth = None;
                    }
                }
                // Restore the mpv child window now that the modal is gone.
                #[cfg(target_os = "linux")]
                player_clone.update(cx, |p, _| p.show_video());
                cx.notify();
            },
        );
        self._subscriptions.push(_sub);
        // Hide mpv so the GPUI modal can render without being covered by
        // the X11 child window.
        #[cfg(target_os = "linux")]
        self.player.update(cx, |p, _| p.hide_video());
        self.auth = Some(auth);
        cx.notify();
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Record this frame + prune entries older than 1s.
        {
            let now = std::time::Instant::now();
            let cutoff = now - std::time::Duration::from_secs(1);
            let mut ft = self.frame_times.borrow_mut();
            ft.push_back(now);
            while ft.front().map_or(false, |t| *t < cutoff) {
                ft.pop_front();
            }
        }
        // Kick another render in ~200ms so the FPS counter keeps ticking
        // even when the rest of the UI is idle. cheap: pure notify.
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(200))
                .await;
            if let Some(e) = this.upgrade() {
                let _ = cx.update_entity(&e, |_, cx| cx.notify());
            }
        })
        .detach();

        let fps = self.frame_times.borrow().len();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0e0e10))
            .text_color(rgb(0xefeff1))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(36.0))
                    .px_3()
                    .bg(rgb(0x18181b))
                    .border_b_1()
                    .border_color(rgb(0x2d2d30))
                    // Left: app title + GitHub link (flex_1 to balance right)
                    .child({
                        let gh_icon = self
                            .icons
                            .borrow_mut()
                            .get(IconName::Github, 15, 0xaaaaaa);
                        let logo = koala_logo();
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(img(logo).w(px(22.0)).h(px(22.0)))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0x9b59b6))
                                    .child("Koala TV"),
                            )
                            .child({
                                let mut link = div()
                                    .id("github-link")
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .on_click(|_, _window, _cx| {
                                        let _ = std::process::Command::new("xdg-open")
                                            .arg("https://github.com/Louisdelez/InteractiveYoutube")
                                            .spawn();
                                    });
                                if let Some(icon) = gh_icon {
                                    link = link.child(img(icon).w(px(15.0)).h(px(15.0)));
                                }
                                link
                            })
                            // Total viewers across all channels — pushed by
                            // the server's `viewers:total` event. Sits to
                            // the right of the GitHub icon.
                            .child({
                                let eye = self
                                    .icons
                                    .borrow_mut()
                                    .get(IconName::Eye, 13, 0xbf94ff);
                                let mut pill = div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .px_2()
                                    .py(px(2.0))
                                    .rounded(px(6.0))
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xbf94ff))
                                    .bg(rgba(0x9b59b61f));
                                if let Some(icon) = eye {
                                    pill = pill.child(img(icon).w(px(13.0)).h(px(13.0)));
                                }
                                pill.child(format!("{}", self.total_viewers))
                            })
                            // Refresh button — F5-like client-only refresh.
                            // No server side effect: the server just gets read
                            // requests it already serves to anyone. Effects:
                            //   1. Re-fetch /api/tv/channels → sidebar rebuild
                            //   2. Ask server to re-emit chat:history for the
                            //      current channel (chat panel resets)
                            //   3. Ask server for a fresh tv:state (player
                            //      resync — loadfile if video changed)
                            //   4. Re-push the anonymous pseudo so the new
                            //      chat:history is rendered with the right
                            //      identity if the user types right after.
                            .child({
                                let icon = self
                                    .icons
                                    .borrow_mut()
                                    .get(IconName::Refresh, 14, 0xaaaaaa);
                                let mut btn = div()
                                    .id("refresh-btn")
                                    .w(px(22.0))
                                    .h(px(22.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .hover(|this| this.bg(rgb(0x26262b)))
                                    .on_click(cx.listener(|this: &mut AppView, _ev: &ClickEvent, _, cx| {
                                        // 1. Re-fetch channels in the background
                                        //    and push them to the sidebar.
                                        let sidebar = this.sidebar.clone();
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
                                            // Wait up to ~3s for the fetch
                                            for _ in 0..30 {
                                                if let Ok(channels) = rx.try_recv() {
                                                    let _ = cx.update(|cx| {
                                                        sidebar.update(cx, |s, cx| {
                                                            s.set_channels_from_server(channels);
                                                            cx.notify();
                                                        });
                                                    });
                                                    return;
                                                }
                                                cx.background_executor().timer(
                                                    std::time::Duration::from_millis(100)
                                                ).await;
                                            }
                                        }).detach();

                                        // 2. Re-pull chat history for current
                                        //    channel + 3. resync TV state.
                                        if let Some(ch) = &this.current_channel_id {
                                            let _ = this.cmd_tx.send(
                                                ClientCommand::ChatChannelChanged(ch.clone())
                                            );
                                        }
                                        let _ = this.cmd_tx.send(ClientCommand::RequestState);

                                        // 4. Re-push our anonymous identity so
                                        //    new outgoing messages keep the
                                        //    same pseudo if we type next.
                                        let pseudo = crate::services::pseudo::get_or_create_pseudo();
                                        let color = crate::services::pseudo::get_or_create_color();
                                        let _ = this.cmd_tx.send(
                                            ClientCommand::SetAnonymousName { name: pseudo, color }
                                        );
                                    }));
                                if let Some(icon) = icon {
                                    btn = btn.child(img(icon).w(px(14.0)).h(px(14.0)));
                                }
                                btn
                            })
                    })
                    // Center: search bar — height tuned so the
                    // gpui-component Input fits inside the 36px topbar
                    // without overflowing (the input widget has its own
                    // internal padding that needs ~28px clear).
                    .child({
                        let search_icon = self
                            .icons
                            .borrow_mut()
                            .get(IconName::Search, 14, 0xaaaaaa);
                        let mut row = div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pl_2()
                            .h(px(28.0))
                            .w(px(280.0))
                            .bg(rgb(0x0e0e10))
                            .border_1()
                            .border_color(rgb(0x2d2d30))
                            .rounded(px(4.0))
                            .overflow_hidden();
                        if let Some(icon) = search_icon {
                            row = row.child(img(icon).w(px(14.0)).h(px(14.0)).flex_none());
                        }
                        // Inner wrapper has explicit width matching the
                        // remaining space so the gpui-component Input
                        // (which has its own internal right-padding +
                        // possible clear button) can't push past the
                        // outer container's rounded right edge.
                        row.child(
                            div()
                                .w(px(248.0))
                                .h_full()
                                .overflow_hidden()
                                .child(Input::new(&self.search_state)),
                        )
                    })
                    // Right: auth status + signal bars + ping
                    .child({
                        let user = self.user.clone();
                        let latency = self.latency_ms;
                        div()
                            .flex_1()
                            .flex()
                            .justify_end()
                            .items_center()
                            .gap_3()
                            // Planning / calendrier — opens the web
                            // planning page in the user's default browser
                            // with the current desktop channel pre-selected
                            // via the hash param. The web page centres the
                            // red "now" line in the viewport on open.
                            .child({
                                let icon = self
                                    .icons
                                    .borrow_mut()
                                    .get(IconName::Calendar, 14, 0xaaaaaa);
                                let mut btn = div()
                                    .id("planning-btn")
                                    .w(px(22.0))
                                    .h(px(22.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .hover(|this| this.bg(rgb(0x26262b)))
                                    .on_click(cx.listener(|this: &mut AppView, _ev: &ClickEvent, window, cx| {
                                        this.open_planning(window, cx);
                                    }));
                                if let Some(icon) = icon {
                                    btn = btn.child(img(icon).w(px(14.0)).h(px(14.0)));
                                }
                                btn
                            })
                            // Chat toggle — show/hide the right sidebar.
                            // Same place + behaviour as the web's chat-toggle.
                            // Sits to the LEFT of the auth pill.
                            .child({
                                let chat_open = self.chat_open;
                                let icon_name = if chat_open {
                                    IconName::MessageSquareOff
                                } else {
                                    IconName::MessageSquare
                                };
                                let icon = self
                                    .icons
                                    .borrow_mut()
                                    .get(icon_name, 16, 0xaaaaaa);
                                let mut btn = div()
                                    .id("chat-toggle")
                                    .w(px(24.0))
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .hover(|this| this.bg(rgb(0x26262b)))
                                    .on_click(cx.listener(|this: &mut AppView, _ev: &ClickEvent, _, cx| {
                                        this.chat_open = !this.chat_open;
                                        let open = this.chat_open;
                                        this.player.update(cx, |p, _| p.set_chat_open(open));
                                        cx.notify();
                                    }));
                                if let Some(icon) = icon {
                                    btn = btn.child(img(icon).w(px(16.0)).h(px(16.0)));
                                }
                                btn
                            })
                            // Auth pill
                            .child(match user {
                                Some(u) => {
                                    let username = u.username.clone();
                                    div()
                                        .id("auth-logout")
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .py_1()
                                        .rounded(px(4.0))
                                        .cursor_pointer()
                                        .hover(|this| this.bg(rgb(0x26262b)))
                                        .text_xs()
                                        .text_color(rgb(0xe8e8ea))
                                        .child(format!("👤 {}", username))
                                        .on_click(cx.listener(|this, _ev: &ClickEvent, _, cx| {
                                            std::thread::spawn(|| {
                                                let _ = api::logout();
                                            });
                                            this.user = None;
                                            this.sidebar.update(cx, |s, cx| {
                                                s.set_logged_in(false);
                                                cx.notify();
                                            });
                                            cx.notify();
                                        }))
                                        .into_any_element()
                                }
                                None => div()
                                    .id("auth-open")
                                    .flex()
                                    .items_center()
                                    .px_3()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .bg(rgb(0x9b59b6))
                                    .hover(|this| this.bg(rgb(0xb57edc)))
                                    .text_xs()
                                    .text_color(rgb(0xffffff))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Connexion")
                                    .on_click(cx.listener(|this, _ev: &ClickEvent, window, cx| {
                                        this.open_auth(window, cx);
                                    }))
                                    .into_any_element(),
                            })
                            // Settings gear icon — opens the settings
                            // modal (memory cache size, purge).
                            .child({
                                let gear = self
                                    .icons
                                    .borrow_mut()
                                    .get(IconName::Settings, 16, 0xaaaaaa);
                                let mut btn = div()
                                    .id("settings-open")
                                    .w(px(24.0))
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .hover(|this| this.bg(rgb(0x26262b)))
                                    .on_click(cx.listener(|this, _ev: &ClickEvent, _, cx| {
                                        this.open_settings(cx);
                                    }));
                                if let Some(icon) = gear {
                                    btn = btn.child(img(icon).w(px(16.0)).h(px(16.0)));
                                }
                                btn
                            })
                            // Signal bars — WiFi-style indicator. The "x ms"
                            // value is surfaced via the shared X11 tooltip on
                            // hover (same widget as the channel name tooltip),
                            // so the topbar stays quiet by default.
                            .child({
                                let tooltip = self.tooltip.clone();
                                let tooltip_leave = self.tooltip.clone();
                                let label = match latency {
                                    Some(ms) => format!("{} ms", ms),
                                    None => "—".to_string(),
                                };
                                div()
                                    .id("ping-indicator")
                                    .child(signal_bars(latency))
                                    .on_hover(move |hovered: &bool, _window, _cx| {
                                        if *hovered {
                                            if let Some(tt) = tooltip.borrow_mut().as_mut() {
                                                if let Some((mx, my)) = tt.query_pointer() {
                                                    tt.show(&label, mx + 14, my + 16);
                                                }
                                            }
                                        } else if let Some(tt) = tooltip_leave.borrow_mut().as_mut() {
                                            tt.hide();
                                        }
                                    })
                            })
                            // App FPS (render calls / second, rolling 1s)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x888888))
                                    .child(format!("{} fps", fps)),
                            )
                    })
            )
            .child({
                let mut row = div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(self.sidebar.clone())
                    .child(self.player.clone());
                if self.chat_open {
                    row = row.child(self.chat.clone());
                }
                row
            })
            // Offline curtain: when the server is unreachable we hide the
            // mpv child window and cover the whole window with a dark overlay
            // so the user can't interact with stale content. Without the
            // server there's no truth to display.
            .child(if !self.connected {
                deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(rgba(0x0e0e10ee))
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_color(rgb(0xef4444))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Serveur indisponible"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x888888))
                                .child("Connexion au serveur perdue. L'application reprendra automatiquement dès que le serveur sera accessible."),
                        )
                        .occlude(),
                )
                .with_priority(8)
                .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(match self.auth.clone() {
                Some(auth) => deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(rgba(0x000000cc))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(auth)
                        .occlude(),
                )
                .with_priority(10)
                .into_any_element(),
                None => div().into_any_element(),
            })
            .child(match self.settings_modal.clone() {
                Some(modal) => deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(rgba(0x000000cc))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(modal)
                        .occlude(),
                )
                .with_priority(11)
                .into_any_element(),
                None => div().into_any_element(),
            })
            .child(match self.planning.clone() {
                Some(planning) => deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(rgb(0x0a0a0d))
                        .child(planning)
                        .occlude(),
                )
                .with_priority(12)
                .into_any_element(),
                None => div().into_any_element(),
            })
    }
}

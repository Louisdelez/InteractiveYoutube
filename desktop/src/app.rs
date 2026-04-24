// Sub-modules live in `src/app/`. Extra `impl AppView` blocks in the
// child modules inherit private-field access to AppView.
mod background_tasks;
mod dispatch;
mod fps;
mod helpers;
mod modals;
mod render;
mod subscriptions;
use fps::FpsCounter;
use helpers::{
    detect_image_format, koala_logo, signal_bars,
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

            // Sidebar click → switch channel (optimistic instant-zap).
            let sub_channel = subscriptions::channel_click(&sidebar, cmd_tx.clone(), cx);
            // mpv auto-advance → refresh tv:state (absorbs drift).
            let sub_advance = subscriptions::auto_advance(&player, cmd_tx.clone(), cx);
            // Channel-badge star click → toggle favourite.
            let sub_badge_fav = subscriptions::badge_favorite(&player, cx);
            // Player memory cache → sidebar "Mémoire" section.
            let sub_memory = subscriptions::memory_changed(&player, sidebar.clone(), cx);

            // X11 tooltip overlay (top-level window above mpv).
            let tooltip = Rc::new(RefCell::new(TooltipOverlay::new()));
            // Debounce counter for delayed tooltip hide.
            let hide_version = Rc::new(std::cell::Cell::new(0u64));

            // Sidebar right-click → toggle favourite.
            let sub_favorite = subscriptions::sidebar_favorite(&sidebar, cx);
            // Sidebar hover → tooltip + debounced preload.
            let sub_hover = subscriptions::channel_hover(
                &sidebar,
                tooltip.clone(),
                hide_version.clone(),
                cx,
            );
            // Chat Enter → send over WebSocket (server echoes back; no local echo).
            let sub_chat = subscriptions::chat_send(&chat, cmd_tx.clone(), cx);

            // Fetch channel list from server in background, then download avatars
            let sidebar_fetch = sidebar.clone();
            let entity_for_avatar = cx.entity().downgrade();
            background_tasks::channels_and_avatars(sidebar_fetch, entity_for_avatar, cx);

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
            background_tasks::state_prefetch(sidebar_prefetch, entity_for_prefetch, cx);

            // Periodically snapshot `last_state_per_channel` to disk so
            // a fresh boot of the app starts with a warm cache. Live
            // tv:state / tv:sync continue to overwrite entries in
            // memory throughout the session; we just push the current
            // view to disk every 30 s. 30 s is long enough to avoid
            // disk thrashing (tv:sync fires every 15 s per channel) and
            // short enough that a crash loses at most half a minute of
            // staleness vs. what we could have persisted.
            let entity_for_save = cx.entity().downgrade();
            background_tasks::state_cache_save(entity_for_save, cx);

            // Poll the server-event channel from GPUI's executor and dispatch to views
            let rx = Arc::new(Mutex::new(event_rx));
            let player_poll = player.clone();
            let chat_poll = chat.clone();
            let entity_for_status = cx.entity().downgrade();
            let cmd_tx_for_pseudo = cmd_tx.clone();
            dispatch::start(rx, player_poll, chat_poll, entity_for_status, cmd_tx_for_pseudo, cx);

            // Ping the server's /health endpoint every 2s to measure latency.
            // The result drives the WiFi-bars indicator + offline overlay.
            let entity_for_ping = cx.entity().downgrade();
            background_tasks::latency_probe(entity_for_ping, cx);

            // Probe /api/auth/me at startup — if a session cookie exists from
            // a previous run we'll be auto-logged-in.
            let entity_for_me = cx.entity().downgrade();
            background_tasks::me_probe(entity_for_me, cx);

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



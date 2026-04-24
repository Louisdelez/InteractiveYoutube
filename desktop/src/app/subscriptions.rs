//! Event subscriptions wired from `AppView::new()`. These are all
//! one-off `cx.subscribe(...)` calls whose only reason to live in the
//! parent was lexical: they touched `sidebar`, `player`, `chat`, and
//! needed access to `AppView` fields via their callbacks. Lifting each
//! one out as a `pub(super) fn` keeps the subscription wiring near the
//! `AppView` impl block but off the already-large `new()`.
//!
//! Each helper returns a `Subscription`; `new()` collects them into the
//! `_subscriptions` vec so they drop with the entity.

use super::*;

/// Sidebar channel click → switch channel on the server + optimistic
/// instant-zap from local `last_state_per_channel` cache.
pub(super) fn channel_click(
    sidebar: &Entity<SidebarView>,
    cmd_tx: std::sync::mpsc::Sender<ClientCommand>,
    cx: &mut Context<AppView>,
) -> Subscription {
    cx.subscribe(
        sidebar,
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
            // previous channel disappear during the switch. Use
            // `clear_messages` (unconditional) rather than
            // `replace_messages(Vec::new(), ...)` — the latter has an
            // anti-reconnect guard that ignores empty histories when
            // local state is non-empty, which silently swallows the
            // intended clear and leaves the old channel's chat on
            // screen until the server round-trip brings a new one.
            this.chat.update(cx, |c, cx| {
                c.clear_messages(cx);
                cx.notify();
            });
            // Instant visual feedback: if this favorite has a
            // pre-fetched thumbnail cached in memory, paint it over
            // the video area while mpv (main + backup) spin up their
            // first frame. Cleared by the poll loop as soon as
            // either becomes the visible surface. Non-favorites fall
            // through (cache miss) — the mpv-based zap still fires
            // and the previous frame stays visible for ~100 ms.
            #[cfg(target_os = "linux")]
            if let Some(entry) = this.frame_cache.get(&event.channel_id) {
                let image = entry.image.clone();
                this.player.update(cx, |p, _| p.show_snapshot(image));
            }
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
            let _ = cmd_tx.send(ClientCommand::SwitchChannel(event.channel_id.clone()));
            // Ask the server for the new channel's chat history.
            let _ = cmd_tx.send(ClientCommand::ChatChannelChanged(event.channel_id.clone()));
        },
    )
}

/// Sidebar channel hover → tooltip + debounced mpv preload.
pub(super) fn channel_hover(
    sidebar: &Entity<SidebarView>,
    tooltip_handle: Rc<RefCell<Option<TooltipOverlay>>>,
    hide_version_handle: Rc<std::cell::Cell<u64>>,
    cx: &mut Context<AppView>,
) -> Subscription {
    cx.subscribe(
        sidebar,
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
                    let _ = id;
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
    )
}

/// Channel badge star click → toggle favourite for the active channel.
pub(super) fn badge_favorite(
    player: &Entity<PlayerView>,
    cx: &mut Context<AppView>,
) -> Subscription {
    cx.subscribe(
        player,
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
                s.set_favorites(favs.clone());
                cx.notify();
            });
            // Keep the frame-snapshot cache scoped to favorites only.
            // Newly-added favorite → fetch its thumbnail now (we
            // already know the videoId from last_state_per_channel).
            // Newly-removed → drop the decoded Image to free RAM.
            sync_frame_cache_to_favorites(this, &id, is_fav, cx);
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
            let _ = is_fav;
        },
    )
}

/// Shared logic for both badge + sidebar favorite toggles : after
/// favorites change, evict the dropped one from the frame cache (free
/// RAM) or fetch a snapshot for the newly-added one.
fn sync_frame_cache_to_favorites(
    this: &mut AppView,
    toggled_channel: &str,
    now_favorite: bool,
    cx: &mut Context<AppView>,
) {
    let favs = this.settings.favorites.clone();
    this.frame_cache.evict_non_favorites(&favs);
    if now_favorite {
        if let Some(state) = this.last_state_per_channel.get(toggled_channel) {
            let video_id = state.video_id.clone();
            super::background_tasks::fetch_snapshot(
                cx.entity().downgrade(),
                toggled_channel.to_string(),
                video_id,
                cx,
            );
        }
    }
}

/// Sidebar right-click → toggle favourite.
pub(super) fn sidebar_favorite(
    sidebar: &Entity<SidebarView>,
    cx: &mut Context<AppView>,
) -> Subscription {
    cx.subscribe(
        sidebar,
        move |this: &mut AppView, _sidebar, event: &ChannelFavoriteToggle, cx| {
            let id = event.0.clone();
            if let Some(pos) = this.settings.favorites.iter().position(|x| x == &id) {
                this.settings.favorites.remove(pos);
            } else {
                this.settings.favorites.push(id.clone());
            }
            settings::save(&this.settings);
            spawn_push_user_settings(this.settings.clone());
            let favs = this.settings.favorites.clone();
            let is_fav = favs.iter().any(|f| f == &id);
            this.sidebar.update(cx, |s, cx| {
                s.set_favorites(favs);
                cx.notify();
            });
            sync_frame_cache_to_favorites(this, &id, is_fav, cx);
        },
    )
}

/// Player memory-cache change → sync the sidebar's "Mémoire" section.
pub(super) fn memory_changed(
    player: &Entity<PlayerView>,
    sidebar_mem: Entity<SidebarView>,
    cx: &mut Context<AppView>,
) -> Subscription {
    cx.subscribe(
        player,
        move |_this: &mut AppView, _player, ev: &MemoryChanged, cx| {
            let ids = ev.0.clone();
            sidebar_mem.update(cx, |s, cx| {
                s.set_memory_channel_ids(ids);
                cx.notify();
            });
        },
    )
}

/// Player auto-advance → request fresh server state (absorbs drift).
pub(super) fn auto_advance(
    player: &Entity<PlayerView>,
    cmd_tx: std::sync::mpsc::Sender<ClientCommand>,
    cx: &mut Context<AppView>,
) -> Subscription {
    cx.subscribe(
        player,
        move |_this: &mut AppView, _player, _ev: &AutoAdvanced, _cx| {
            let _ = cmd_tx.send(ClientCommand::RequestState);
        },
    )
}

/// Chat Enter → send to server (server echoes back; no local echo).
pub(super) fn chat_send(
    chat: &Entity<ChatView>,
    cmd_tx: std::sync::mpsc::Sender<ClientCommand>,
    cx: &mut Context<AppView>,
) -> Subscription {
    cx.subscribe(
        chat,
        move |_this: &mut AppView, _chat, event: &ChatSend, _cx| {
            let _ = cmd_tx.send(ClientCommand::SendChat(event.text.clone()));
        },
    )
}

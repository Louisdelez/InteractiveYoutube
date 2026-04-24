//! Server-event dispatch loop. Pulls `ServerEvent`s from the
//! websocket's mpsc channel, matches them, and forwards to the
//! appropriate view entity. Extracted from `AppView::new()` so the
//! parent file shrinks toward the <500-LOC target.
//!
//! Captures (all passed into `start()` as arguments):
//!   - rx                    Arc<Mutex<Receiver<ServerEvent>>>
//!   - player_poll           Entity<PlayerView>
//!   - chat_poll             Entity<ChatView>
//!   - entity_for_status     WeakEntity<AppView>
//!   - cmd_tx_for_pseudo     Sender<ClientCommand> (for reconnect re-assert)
//!
//! Body is verbatim from the original inline cx.spawn block.

use super::*;

pub(super) fn start(
    rx: Arc<Mutex<std::sync::mpsc::Receiver<ServerEvent>>>,
    player_poll: Entity<PlayerView>,
    chat_poll: Entity<ChatView>,
    entity_for_status: WeakEntity<AppView>,
    cmd_tx_for_pseudo: std::sync::mpsc::Sender<ClientCommand>,
    cx: &mut Context<AppView>,
) {
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
                                                e.update(cx, |this, cx| {
                                                    this.last_state_per_channel
                                                        .insert(state.channel_id.clone(), state.clone());
                                                    super::background_tasks::broadcast_remote_state(this, cx);
                                                    // If this channel is a favorite and
                                                    // its thumbnail cache is missing or
                                                    // pointing at a stale videoId, kick
                                                    // off a background fetch so the
                                                    // next click paints a snapshot
                                                    // instead of the previous frame.
                                                    let is_fav = this.settings.favorites.iter().any(|f| f == &state.channel_id);
                                                    let needs = is_fav && this.frame_cache.needs_refresh(&state.channel_id, &state.video_id);
                                                    if needs {
                                                        super::background_tasks::fetch_snapshot(
                                                            cx.entity().downgrade(),
                                                            state.channel_id.clone(),
                                                            state.video_id.clone(),
                                                            cx,
                                                        );
                                                    }
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
}

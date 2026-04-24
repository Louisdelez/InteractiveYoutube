//! The `Render` impl for `AppView`. Extracted from the main `app.rs`
//! so the parent file shrinks toward the audit's <500 LOC target.
//!
//! Child module of `app`, inherits private-field access. All state
//! reads go directly against `self.<field>`.

use super::*;

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame_times.record();
        let fps = self.frame_times.current();
        FpsCounter::schedule_next_tick(cx);
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
                                    .h(px(22.0))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(rgb(0xaaaaaa))
                                    .hover(|this| this.bg(rgb(0x26262b)).text_color(rgb(0xefeff1)))
                                    .on_click(cx.listener(|this: &mut AppView, _ev: &ClickEvent, window, cx| {
                                        this.open_planning(window, cx);
                                    }));
                                if let Some(icon) = icon {
                                    btn = btn.child(img(icon).w(px(14.0)).h(px(14.0)));
                                }
                                btn.child(t("topbar.programme.label"))
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
                                    .child(crate::i18n::t("topbar.connect.label"))
                                    .on_click(cx.listener(|this, _ev: &ClickEvent, window, cx| {
                                        this.open_auth(window, cx);
                                    }))
                                    .into_any_element(),
                            })
                            // Settings gear icon — opens the settings
                            // modal (memory cache size, purge).
                            // Remote control — opens the QR-code pairing
                            // modal. Placed just before the gear so the
                            // "infrastructure / config" group stays together.
                            .child({
                                let remote_icon = self
                                    .icons
                                    .borrow_mut()
                                    .get(IconName::Remote, 16, 0xaaaaaa);
                                let active = self.remote_modal.is_some();
                                let bg = if active { rgb(0x9b59b6) } else { rgb(0x18181b) };
                                let mut btn = div()
                                    .id("remote-open")
                                    .w(px(24.0))
                                    .h(px(24.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .bg(bg)
                                    .hover(|this| this.bg(rgb(0x26262b)))
                                    .on_click(cx.listener(|this, _ev: &ClickEvent, _, cx| {
                                        this.toggle_remote(cx);
                                    }));
                                if let Some(icon) = remote_icon {
                                    btn = btn.child(img(icon).w(px(16.0)).h(px(16.0)));
                                }
                                btn
                            })
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
                            // App FPS — render calls / second rolling 1 s.
                            // Diagnostic: if it drops below ~30 during
                            // playback, something's contending the UI
                            // thread.
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
                                .child(t("status.server_down")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x888888))
                                .child(t("status.server_down_body")),
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
            .child(match self.remote_modal.clone() {
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
            .child(if self.maintenance_warning && !self.maintenance {
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(32.0))
                    .bg(rgb(0x9b59b6))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xffffff))
                    .child(t("maintenance.warning_banner"))
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child(if self.maintenance {
                deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(gpui::rgba(0x000000cc))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .px_6()
                                .py_4()
                                .bg(rgb(0x1f1f23))
                                .rounded(px(12.0))
                                .border_1()
                                .border_color(rgb(0x9b59b6))
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0x9b59b6))
                                        .child(t("maintenance.ongoing_title"))
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(0xaaaaaa))
                                        .child(crate::i18n::t("status.service_starting"))
                                )
                        )
                        .occlude(),
                )
                .with_priority(20)
                .into_any_element()
            } else {
                div().into_any_element()
            })
    }
}

//! The `Render` impl for `PlayerView`. Extracted from the main
//! `views/player.rs` so that file can shrink toward the audit's
//! <800-LOC target. The render tree is ~270 LOC of GPUI flex-box
//! composition; keeping it here means changes to visual layout
//! don't churn the state-bearing parent file.
//!
//! Child module of `views::player`, so it inherits private-field
//! access to `PlayerView` and reads fields directly.

use super::*;

impl Render for PlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Compute the loading state up-front so we can both gate the mpv
        // child window and decide whether to draw the spinner overlay.
        // ── Channel-switch loading-overlay state machine ─────────────
        // Industry-standard "delayed spinner" pattern:
        //  • 0–400 ms after click: nothing (previous frame stays).
        //  • 400 ms+ if backup not yet rendering: spinner appears.
        //  • Once spinner is shown: keep visible at least 500 ms.
        //  • Once backup is rendering AND min duration met: overlay
        //    disappears, backup is revealed → fluid transition.
        // Loading overlay disabled — was causing more problems than it
        // solved. mpv plays as it always did; no visual overlay.
        self.switch_arm_at = None;
        self.switch_overlay_shown_at = None;
        self.switch_backup_ready_at = None;
        let is_loading = false;

        #[cfg(target_os = "linux")]
        {
            let vs = window.viewport_size();
            let chat_w = if self.chat_open { CHAT_W } else { 0.0 };
            let w = (f32::from(vs.width) - SIDEBAR_W - chat_w).max(100.0) as u32;
            let h = (f32::from(vs.height) - TOPBAR_H - CONTROL_BAR_H - INFOBAR_H).max(100.0) as u32;
            // mpv is at the on-screen position when no modal is open,
            // off-screen when a modal hides it (so GPUI overlays
            // aren't covered by mpv's X11 child window).
            if self.video_hidden {
                self.apply_geometry(-10000, -10000, w, h);
            } else {
                self.apply_geometry(SIDEBAR_W as i32, TOPBAR_H as i32, w, h);
            }

            // Keep the overlay sized to mpv's area. Show/hide it based
            // on the switch state machine — purely visual, mpv is
            // never touched.
            if let Some(ov) = self.loading_overlay.as_mut() {
                ov.set_geometry(SIDEBAR_W as i32, TOPBAR_H as i32, w, h);
                if is_loading {
                    ov.show();
                } else if ov.is_visible() {
                    ov.hide();
                }
            }

            // "Now playing" badge — flush to the top-left corner of
            // the video area, raised above mpv via X11 stacking.
            // Auto-hides 4s after a channel switch (Apple TV / YT TV
            // pattern); the poll loop bumps the timer whenever the
            // mouse hovers over the video area, so any user
            // interaction makes it re-appear.
            if let Some(badge) = self.channel_badge.as_mut() {
                if self.video_hidden {
                    badge.hide();
                } else {
                    badge.place(SIDEBAR_W as i32, TOPBAR_H as i32);
                    if badge.should_be_visible() {
                        badge.show();
                    } else if badge.is_visible() {
                        badge.hide();
                    }
                }
                cx.notify();
            }

            // While loading: keep mpv mapped but pushed off-screen so its
            // decoder keeps running at full speed (XUnmap stalls it).
            // The black GPUI overlay + spinner shows in the visible area.
            // After loading: apply_geometry above already restored the
            // on-screen position, so mpv main is visible. If the swap
            // logic put us on backup, raise it on top.
            if !is_loading {
                if let Some(b) = self.backup.as_mut() {
                    if self.using_backup {
                        b.show();
                    }
                }
            }
            // Re-render automatically while loading so the deadline check
            // flips us out of loading on its own (no need for an external
            // poke).
            if is_loading {
                cx.notify();
            }

            // Track control-bar geometry in window coords so popup menus can
            // anchor to it when opened from GPUI click handlers.
            self.control_bar_y = TOPBAR_H as i32 + h as i32;
            self.control_bar_right = SIDEBAR_W as i32 + w as i32;
        }

        let volume = self.volume;
        let sub_label = self.sub_label.clone();
        let audio_label = self.audio_label.clone();
        let quality_label = QUALITIES[self.quality_idx].0.to_string();
        let captions_on = self.captions_on;

        // Pre-compute icon images (cached after first render)
        let play_icon = self.icons.get(IconName::Play, ICON_PX, TEXT_PRIMARY);
        let vol_icon_name = if volume == 0 { IconName::VolumeMute } else { IconName::Volume };
        let vol_icon_color = if volume == 0 { TEXT_MUTED } else { TEXT_PRIMARY };
        let vol_icon = self.icons.get(vol_icon_name, ICON_PX, vol_icon_color);
        let cc_icon_name = if captions_on { IconName::Captions } else { IconName::CaptionsOff };
        let cc_icon_color = if captions_on { ACCENT } else { TEXT_PRIMARY };
        let cc_icon = self.icons.get(cc_icon_name, ICON_PX, cc_icon_color);
        let audio_icon = self.icons.get(IconName::Languages, ICON_PX, TEXT_PRIMARY);
        let qual_icon = self.icons.get(IconName::Settings, ICON_PX, TEXT_PRIMARY);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .bg(rgb(0x000000))
            .child({
                // The video area: black background. When loading, show an
                // animated spinner centered. When not loading, this stays
                // empty so mpv (overlaid via X11 child window) is visible.
                let mut area = div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0x000000));
                if is_loading {
                    area = area.child(loading_indicator());
                }
                area
            })
            // ── Modern playback bar ────────────────────────────────────────
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_4()
                    .h(px(CONTROL_BAR_H))
                    .bg(rgb(BAR_BG))
                    .border_t_1()
                    .border_color(rgb(BAR_BORDER))
                    // Play
                    .child(icon_button(
                        "force-play",
                        play_icon,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, _, _| this.force_play()),
                    ))
                    // Volume icon (mute toggle)
                    .child(icon_button(
                        "vol-icon",
                        vol_icon,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, window, cx| {
                            let new = if this.volume > 0 { 0 } else { 100 };
                            this.volume = new;
                            mpv_try!(this.mpv.set_property("volume", new), "main volume slider", new);
                            this.volume_state.update(cx, |s, cx| {
                                s.set_value(new as f32, window, cx);
                            });
                        }),
                    ))
                    // Volume slider
                    .child(
                        div()
                            .ml_1()
                            .w(px(96.0))
                            .child(Slider::new(&self.volume_state).horizontal()),
                    )
                    // Volume %
                    .child(
                        div()
                            .ml_2()
                            .w(px(32.0))
                            .text_xs()
                            .text_color(rgb(TEXT_MUTED))
                            .child(format!("{}%", volume)),
                    )
                    // Spacer
                    .child(div().flex_1())
                    // Captions trigger (opens X11 popup above)
                    .child(icon_label_button(
                        "captions",
                        cc_icon,
                        &sub_label,
                        captions_on,
                        cx.listener(|this, _ev: &ClickEvent, _, _| {
                            #[cfg(target_os = "linux")]
                            this.toggle_popup(MenuKind::Captions);
                        }),
                    ))
                    // Audio trigger
                    .child(icon_label_button(
                        "audio",
                        audio_icon,
                        &audio_label,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, _, _| {
                            #[cfg(target_os = "linux")]
                            this.toggle_popup(MenuKind::Audio);
                        }),
                    ))
                    // Quality trigger
                    .child(icon_label_button(
                        "quality",
                        qual_icon,
                        &quality_label,
                        false,
                        cx.listener(|this, _ev: &ClickEvent, _, _| {
                            #[cfg(target_os = "linux")]
                            this.toggle_popup(MenuKind::Quality);
                        }),
                    )),
            )
            // ── Info bar (title + YouTube link) ───────────────────────────
            .child({
                let yt_icon = self.icons.get(IconName::Youtube, 16, 0xff0000);
                let video_id = self.current_video_id.clone();
                let mut bar = div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .h(px(INFOBAR_H))
                    .bg(rgb(0x18181b))
                    .border_t_1()
                    .border_color(rgb(BAR_BORDER))
                    .child({
                        let date_label = self.published_at.as_deref()
                            .and_then(format_published_tooltip);
                        let mut col = div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(TEXT_PRIMARY))
                                    .child(self.title.clone())
                            );
                        if let Some(label) = date_label {
                            col = col.child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(TEXT_MUTED))
                                    .child(label)
                            );
                        }
                        col
                    });
                if let Some(vid) = video_id {
                    let url = format!("https://www.youtube.com/watch?v={}", vid);
                    let mut link = div()
                        .id("yt-link")
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_1()
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .hover(|this| this.bg(rgb(BTN_HOVER)))
                        .text_xs()
                        .text_color(rgb(TEXT_MUTED))
                        .on_click(move |_ev: &ClickEvent, _, _| {
                            open_in_browser(&url);
                        });
                    if let Some(icon) = yt_icon {
                        link = link.child(img(icon).w(px(16.0)).h(px(16.0)));
                    }
                    bar = bar.child(link.child(div().child(crate::i18n::t("player.watch_on_youtube"))));
                }
                bar
            })
    }
}

//! `open_settings`, `open_planning`, `open_auth` modal entry points.
//! Extracted from the main `app.rs` so the file stays focused on the
//! AppView::new() setup graph.
//!
//! Child module of `app` so it inherits private access to AppView
//! fields (settings_modal, auth, planning, player, chat, etc.) —
//! no pub(super) shuffling needed on struct fields.

use super::AppView;
use super::{
    api, settings, AuthEvent, AuthView, ChatSend, PlanningClose, PlanningView,
    Settings, SettingsEvent, SettingsModal,
};
use super::helpers::{spawn_pull_user_settings, spawn_push_user_settings};
use gpui::*;

impl AppView {
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
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
                    SettingsEvent::QualityChanged(idx) => {
                        this.settings.preferred_quality = idx;
                        settings::save(&this.settings);
                        spawn_push_user_settings(this.settings.clone());
                        player_clone.update(cx, |p, cx| p.set_quality(idx as usize, cx));
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

    /// Toggle the remote-control modal. Clicking the top-bar button
    /// a second time while open closes it.
    pub(super) fn toggle_remote(&mut self, cx: &mut Context<Self>) {
        if self.remote_modal.is_some() {
            self.close_remote(cx);
            return;
        }
        use crate::views::remote_modal::{RemoteModal, RemoteModalEvent};
        let remote_url = self.remote_server.as_ref().map(|s| s.url.clone());
        let modal = cx.new(|_| RemoteModal::new(remote_url));
        self.remote_modal_sub = Some(cx.subscribe(
            &modal,
            move |this: &mut AppView, _modal, ev: &RemoteModalEvent, cx| match ev {
                RemoteModalEvent::Close => this.close_remote(cx),
            },
        ));
        // Hide mpv X11 child so the modal isn't covered by the X11
        // video surface (same pattern as open_settings).
        #[cfg(target_os = "linux")]
        self.player.update(cx, |p, _| p.hide_video());
        self.remote_modal = Some(modal);
        cx.notify();
    }

    fn close_remote(&mut self, cx: &mut Context<Self>) {
        #[cfg(target_os = "linux")]
        self.player.update(cx, |p, _| p.show_video());
        self.remote_modal = None;
        self.remote_modal_sub = None;
        cx.notify();
    }

    pub(super) fn open_planning(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn open_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

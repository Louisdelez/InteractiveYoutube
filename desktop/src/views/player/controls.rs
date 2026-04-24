//! Captions / audio / quality track listing + popup menu handlers.
//! Extracted from the main `views/player.rs` so the top-level file stays
//! focused on struct definitions + the `new()` setup + render. This
//! module is a child of `views::player` (filesystem-wise:
//! `views/player/controls.rs`) so it inherits private-field access to
//! `PlayerView` — no `pub(super)` fiddling needed.

use super::PlayerView;
use super::{lang_display_name, log_quality, COMMON_SUB_LANGS, QUALITIES};
#[cfg(target_os = "linux")]
use super::{MenuEvent, MenuKind};
use crate::mpv_try;
use gpui::Context;

impl PlayerView {
    pub(super) fn list_sub_tracks(&self) -> Vec<(i64, String)> {
        self.list_all_sub_tracks_filtered(!self.show_all_sub_langs)
    }

    pub(super) fn list_all_sub_tracks_filtered(&self, common_only: bool) -> Vec<(i64, String)> {
        let mpv = &self.mpv;
        let count = mpv.get_property::<i64>("track-list/count").unwrap_or(0);
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut tracks = Vec::new();
        for i in 0..count {
            let ty = mpv
                .get_property::<String>(&format!("track-list/{}/type", i))
                .unwrap_or_default();
            if ty != "sub" {
                continue;
            }
            let id = match mpv.get_property::<i64>(&format!("track-list/{}/id", i)) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let lang = mpv
                .get_property::<String>(&format!("track-list/{}/lang", i))
                .unwrap_or_default();
            if lang.is_empty() {
                continue;
            }
            // "fr-CA" or "aa-fr" → keep the base "fr" / "aa" only (dedupe groups).
            let base = lang.split('-').next().unwrap_or(&lang).to_string();
            if common_only && !COMMON_SUB_LANGS.contains(&base.as_str()) {
                continue;
            }
            if !seen.insert(base.clone()) {
                continue; // already have a track for this base lang
            }
            let label = lang_display_name(&base).to_string();
            tracks.push((id, label));
        }
        // Sort: common langs in fixed order, others alphabetically by label.
        tracks.sort_by(|a, b| a.1.cmp(&b.1));
        tracks
    }

    /// Enumerate audio tracks from mpv's track-list.
    pub(super) fn list_audio_tracks(&self) -> Vec<(i64, String)> {
        let mpv = &self.mpv;
        let count = mpv.get_property::<i64>("track-list/count").unwrap_or(0);
        let mut tracks = Vec::new();
        for i in 0..count {
            let ty = mpv
                .get_property::<String>(&format!("track-list/{}/type", i))
                .unwrap_or_default();
            if ty != "audio" {
                continue;
            }
            let id = match mpv.get_property::<i64>(&format!("track-list/{}/id", i)) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let lang = mpv
                .get_property::<String>(&format!("track-list/{}/lang", i))
                .unwrap_or_default();
            let title = mpv
                .get_property::<String>(&format!("track-list/{}/title", i))
                .unwrap_or_default();
            let label = if !lang.is_empty() {
                lang
            } else if !title.is_empty() {
                title
            } else {
                crate::i18n::t_args("player.audio_track_fmt", &[("id", &id.to_string())])
            };
            tracks.push((id, label));
        }
        tracks
    }


    pub fn set_quality(&mut self, idx: usize, _cx: &mut Context<Self>) {
        if idx >= QUALITIES.len() {
            return;
        }
        self.quality_idx = idx;
        let (_, fmt) = QUALITIES[idx];

        // Save current playback position so we can resume there.
        let saved_pos = self
            .mpv
            .get_property::<f64>("time-pos")
            .unwrap_or(0.0);

        // Step 1 — bring the (already-decoded, already-running) backup
        // mpv to the foreground IMMEDIATELY so the user sees no
        // interruption while the main re-buffers the new quality.
        #[cfg(target_os = "linux")]
        if let Some(b) = self.backup.as_mut() {
            if saved_pos > 0.5 {
                b.seek(saved_pos);
            }
            b.show();
            self.using_backup = true;
            self.backup_since = Some(std::time::Instant::now());
            log_quality(&format!("quality switch → showing backup at t={:.1}s", saved_pos));
        }

        // Step 2 — reload the main with the new ytdl-format. This stalls
        // the main mpv for a few seconds while it re-resolves and buffers,
        // but the user is now watching the backup so doesn't notice.
        {
            let mpv = &self.mpv;
            mpv_try!(mpv.set_property("ytdl-format", fmt), "main quality change", fmt);
            mpv_try!(mpv.set_property("mute", true), "main mute for quality-change reload"); // backup carries the audio
            if saved_pos > 0.5 {
                mpv_try!(
                    mpv.set_property("start", format!("+{}", saved_pos)),
                    "main set start (quality reload)",
                    saved_pos
                );
            }
            mpv_try!(
                mpv.command("loadfile", &[&self.current_url]),
                "main loadfile (quality reload)",
                &self.current_url
            );
        }

        // The poll loop's existing "swap back after 8 s if main is healthy"
        // logic will hide the backup once the new quality is buffered.
    }

    /// Set a specific subtitle track by id (None = off).
    pub fn set_sub_track(&mut self, id: Option<i64>) {
        {
            let mpv = &self.mpv;
            match id {
                Some(sid) => {
                    mpv_try!(mpv.set_property("sid", sid), "main set sub track", sid);
                    mpv_try!(mpv.set_property("sub-visibility", true), "main subs on");
                }
                None => {
                    mpv_try!(mpv.set_property("sub-visibility", false), "main subs off");
                }
            }
        }
        self.captions_on = id.is_some();
        self.sub_label = match id {
            Some(sid) => self
                .list_sub_tracks()
                .into_iter()
                .find(|(tid, _)| *tid == sid)
                .map(|(_, l)| l)
                .unwrap_or_else(|| "On".to_string()),
            None => "Off".to_string(),
        };
    }

    #[cfg(target_os = "linux")]
    pub(super) fn handle_menu_event(&mut self, ev: MenuEvent, cx: &mut Context<Self>) {
        match ev {
            MenuEvent::Selected { kind, index } => match kind {
                MenuKind::Quality => {
                    self.set_quality(index, cx);
                }
                MenuKind::Captions => {
                    let tracks = self.list_sub_tracks();
                    let toggle_idx = tracks.len() + 1; // index of the "more/less" toggle
                    if index == 0 {
                        self.set_sub_track(None);
                    } else if index == toggle_idx {
                        // Toggle expanded list and reopen the popup
                        self.show_all_sub_langs = !self.show_all_sub_langs;
                        if let Some(pop) = self.popup.as_ref() {
                            pop.borrow_mut().close();
                        }
                        self.captions_open = false;
                        self.toggle_popup(MenuKind::Captions);
                        return;
                    } else if let Some((id, _)) = tracks.get(index - 1) {
                        self.set_sub_track(Some(*id));
                    }
                    // Reset to compact view after picking a language
                    self.show_all_sub_langs = false;
                }
                MenuKind::Audio => {
                    let tracks = self.list_audio_tracks();
                    if let Some((id, _)) = tracks.get(index) {
                        self.set_audio_track(*id);
                    }
                }
            },
        }
        if let Some(pop) = self.popup.as_ref() {
            pop.borrow_mut().close();
        }
        self.captions_open = false;
        self.audio_open = false;
        self.quality_open = false;
    }

    #[cfg(target_os = "linux")]
    pub(super) fn toggle_popup(&mut self, kind: MenuKind) {
        let Some(pop) = self.popup.clone() else { return };
        let mut pop = pop.borrow_mut();
        // Close if already showing this menu
        if pop.is_visible() && pop.current_kind() == kind {
            pop.close();
            self.captions_open = false;
            self.audio_open = false;
            self.quality_open = false;
            return;
        }

        let (items, selected): (Vec<String>, Option<usize>) = match kind {
            MenuKind::Quality => {
                let items = QUALITIES.iter().map(|(l, _)| l.to_string()).collect();
                (items, Some(self.quality_idx))
            }
            MenuKind::Captions => {
                let mut items = vec![crate::i18n::t("player.captions_off")];
                let tracks = self.list_sub_tracks();
                let current_sid = self.mpv.get_property::<i64>("sid").ok();
                let mut selected = if self.captions_on { None } else { Some(0) };
                for (i, (id, label)) in tracks.iter().enumerate() {
                    items.push(label.clone());
                    if self.captions_on && current_sid == Some(*id) {
                        selected = Some(i + 1);
                    }
                }
                // Append "Plus de langues" toggle when in compact mode AND
                // there are more languages available than the 5 common ones.
                if !self.show_all_sub_langs {
                    let total = self.list_all_sub_tracks_filtered(false).len();
                    if total > tracks.len() {
                        items.push(crate::i18n::t("player.captions_more_langs"));
                    }
                } else {
                    items.push(crate::i18n::t("player.captions_fewer_langs"));
                }
                (items, selected)
            }
            MenuKind::Audio => {
                let tracks = self.list_audio_tracks();
                let current_aid = self.mpv.get_property::<i64>("aid").ok();
                let items: Vec<String> = if tracks.is_empty() {
                    vec![crate::i18n::t("player.audio_none")]
                } else {
                    tracks.iter().map(|(_, l)| l.clone()).collect()
                };
                let selected = tracks.iter().position(|(id, _)| current_aid == Some(*id));
                (items, selected)
            }
        };

        let anchor_x = self.control_bar_right;
        let anchor_y = self.control_bar_y;
        pop.open(kind, items, selected, anchor_x, anchor_y);
        self.captions_open = matches!(kind, MenuKind::Captions);
        self.audio_open = matches!(kind, MenuKind::Audio);
        self.quality_open = matches!(kind, MenuKind::Quality);
    }

    pub fn set_audio_track(&mut self, id: i64) {
        {
            let mpv = &self.mpv;
            mpv_try!(mpv.set_property("aid", id), "main set audio track", id);
        }
        self.audio_label = self
            .list_audio_tracks()
            .into_iter()
            .find(|(tid, _)| *tid == id)
            .map(|(_, l)| l)
            .unwrap_or_default();
    }
}

//! Settings modal — opened from the gear icon in the topbar.
//!
//! Currently exposes:
//! - **Mémoire**: how many channels to keep "warm" for instant zap
//!   (0 = disabled, 2 = current + 1 previous, up to 5).
//! - **Purger** : drop all cached channels right now.
//!
//! Settings persist locally via `services::settings::save`. When a
//! user is logged in, AppView also pushes them to the server.

use crate::i18n::t;
use crate::services::settings::Settings;
use gpui::*;

#[derive(Clone, Debug)]
pub enum SettingsEvent {
    /// User changed `memory_capacity`. AppView updates PlayerView
    /// + persists.
    MemoryCapacity(u8),
    /// User clicked the "Purger" button. AppView asks PlayerView to
    /// drop all cached channels.
    PurgeMemory,
    /// User changed the preferred max quality. Index into `QUALITIES`
    /// (0=Auto, 1=1080p, 2=720p, 3=480p, 4=360p). AppView forwards
    /// to PlayerView::set_quality and persists.
    QualityChanged(u8),
    /// User closed the modal.
    Close,
}

impl EventEmitter<SettingsEvent> for SettingsModal {}

pub struct SettingsModal {
    pub settings: Settings,
}

impl SettingsModal {
    pub fn new(settings: Settings) -> Self {
        Self { settings }
    }

    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }
}

impl Render for SettingsModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cap = self.settings.memory_capacity;
        let quality = self.settings.preferred_quality;
        let options: [(u8, &'static str); 5] = [
            (0, "Off"),
            (2, "2"),
            (3, "3"),
            (4, "4"),
            (5, "5"),
        ];
        let quality_options: [(u8, &'static str); 5] = [
            (0, "Auto"),
            (1, "1080p"),
            (2, "720p"),
            (3, "480p"),
            (4, "360p"),
        ];

        div()
            .flex()
            .flex_col()
            .w(px(420.0))
            .bg(rgb(0x18181b))
            .border_1()
            .border_color(rgb(0x2d2d30))
            .rounded(px(8.0))
            .shadow_lg()
            .p_5()
            .gap_4()
            // Header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xefeff1))
                            .child(t("settings.title")),
                    )
                    .child(
                        div()
                            .id("settings-close")
                            .text_xs()
                            .text_color(rgb(0xaaaaaa))
                            .cursor_pointer()
                            .hover(|this| this.text_color(rgb(0xefeff1)))
                            .child("✕")
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SettingsEvent::Close);
                            })),
                    ),
            )
            // Memory capacity section
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xefeff1))
                            .child(t("settings.memory.title")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9b9b9e))
                            .child(
                                "Garde N chaînes en mémoire pour zapper sans temps de chargement. \
                                 Inclut la chaîne actuelle."
                            ),
                    )
                    // Option pills
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mt_1()
                            .children(options.iter().map(|(value, label)| {
                                let is_selected = *value == cap;
                                let v = *value;
                                div()
                                    .id(("mem-cap", v as usize))
                                    .px_3()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .bg(if is_selected { rgb(0x9b59b6) } else { rgb(0x26262b) })
                                    .text_color(if is_selected { rgb(0xffffff) } else { rgb(0xefeff1) })
                                    .hover(|this| this.bg(rgb(0xb57edc)))
                                    .child(label.to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.memory_capacity = v;
                                        cx.emit(SettingsEvent::MemoryCapacity(v));
                                        cx.notify();
                                    }))
                            }).collect::<Vec<_>>())
                    )
                    // Resource warning when > 2
                    .child({
                        let warn = match cap {
                            0 => "Désactivé — aucun cache, retour à la chaîne précédente prendra 1-3 s.".to_string(),
                            2 => "Recommandé — ~50-100 MB extra, ~5 % CPU.".to_string(),
                            n => format!(
                                "{} chaînes — ~{} MB extra, ~{} % CPU continu (consomme aussi de la bande passante).",
                                n,
                                ((n as u32 - 1) * 100),
                                ((n as u32 - 1) * 5),
                            ),
                        };
                        div()
                            .text_xs()
                            .text_color(if cap > 2 { rgb(0xeab308) } else { rgb(0x6b7280) })
                            .child(warn)
                    })
                    .child(
                        div()
                            .id("purge-memory")
                            .mt_2()
                            .px_3()
                            .py_1()
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(rgb(0xef4444))
                            .border_1()
                            .border_color(rgb(0x44181c))
                            .bg(rgb(0x1a0e0f))
                            .hover(|this| this.bg(rgb(0x2a1416)))
                            .child(t("settings.memory.purge"))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SettingsEvent::PurgeMemory);
                            })),
                    ),
            )
            // Quality section
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xefeff1))
                            .child(t("settings.quality.title")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x9b9b9e))
                            .child(
                                "Qualité maximale du flux haute résolution. \
                                 N'affecte pas la vitesse de zap (le flux 360p rapide mène toujours)."
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mt_1()
                            .children(quality_options.iter().map(|(value, label)| {
                                let is_selected = *value == quality;
                                let v = *value;
                                div()
                                    .id(("quality", v as usize))
                                    .px_3()
                                    .py_1()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .bg(if is_selected { rgb(0x9b59b6) } else { rgb(0x26262b) })
                                    .text_color(if is_selected { rgb(0xffffff) } else { rgb(0xefeff1) })
                                    .hover(|this| this.bg(rgb(0xb57edc)))
                                    .child(label.to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.preferred_quality = v;
                                        cx.emit(SettingsEvent::QualityChanged(v));
                                        cx.notify();
                                    }))
                            }).collect::<Vec<_>>())
                    )
                    .child({
                        let hint = match quality {
                            0 => "Auto — tente 1080p, redescend si indisponible. Meilleur visuel.",
                            1 => "1080p — qualité max. ~27 % CPU continu sur GTX 1630.",
                            2 => "720p — ~5-8 % CPU de moins, qualité encore excellente.",
                            3 => "480p — léger mais visible sur grand écran.",
                            4 => "360p — minimal CPU / bande passante. Pour WiFi faible.",
                            _ => "",
                        };
                        div()
                            .text_xs()
                            .text_color(rgb(0x6b7280))
                            .child(hint.to_string())
                    }),
            )
    }
}

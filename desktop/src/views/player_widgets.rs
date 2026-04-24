//! GPUI widget / animation helpers extracted from `views/player.rs`.
//! These are pure-GPUI building blocks (icon buttons, loading spinner,
//! volume crossfade spawner) with no `PlayerView` state coupling —
//! they can live outside the 2000-LOC player module.
//!
//! Also helps tests compile: the top-level player.rs is large enough
//! that appending a `#[cfg(test)]` block to it crashes rustc with
//! SIGSEGV on macro recursion. Moving widgets here pre-emptively
//! releases some of that budget.

use crate::services::mpv_ipc::MpvIpcClient;
use crate::theme::colors::{ACCENT, BTN_ACTIVE, BTN_HOVER, TEXT_PRIMARY};
use gpui::*;
use std::sync::Arc;

/// Default icon size (px) used everywhere in the player chrome.
pub const ICON_PX: u32 = 20;

/// Ramp mpv's volume from `from` to `to` over `total_ms` via a detached
/// async task stepping every 16 ms (~60 Hz vsync) — our channel-switch
/// audio crossfade. `mpv` is a clone of the `MpvIpcClient` (Arc-
/// wrapped internally, cheap to clone). Silent failures on the IPC
/// socket are swallowed: the fade is cosmetic, losing a couple of
/// intermediate volume writes just means a slightly steppier curve.
pub fn fade_volume<T: 'static>(
    mpv: MpvIpcClient,
    from: i64,
    to: i64,
    total_ms: u64,
    cx: &mut Context<T>,
) {
    let start = std::time::Instant::now();
    let total = std::time::Duration::from_millis(total_ms);
    cx.spawn(async move |_, cx| {
        let from_f = from as f64;
        let to_f = to as f64;
        loop {
            let elapsed = start.elapsed();
            if elapsed >= total {
                let _ = mpv.set_property("volume", to);
                break;
            }
            let t = elapsed.as_millis() as f64 / total_ms.max(1) as f64;
            let v = (from_f + (to_f - from_f) * t).round() as i64;
            let _ = mpv.set_property("volume", v);
            cx.background_executor()
                .timer(std::time::Duration::from_millis(16))
                .await;
        }
    })
    .detach();
}

/// Centered pulsing violet disc shown while a channel's first frame
/// is loading. Uses GPUI's `Animation` with a triangle-wave easing to
/// ramp opacity 0.25 → 1.0 → 0.25 over 1100 ms. Label is the
/// "Chargement…" string.
pub fn loading_indicator() -> impl IntoElement {
    use gpui::{ease_in_out, Animation, AnimationExt as _};
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .w(px(36.0))
                .h(px(36.0))
                .rounded_full()
                .bg(rgb(ACCENT))
                .with_animation(
                    "loading-pulse",
                    Animation::new(std::time::Duration::from_millis(1100))
                        .repeat()
                        .with_easing(ease_in_out),
                    |this, t| {
                        let tri = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
                        this.opacity(0.25 + 0.75 * tri)
                    },
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(crate::theme::colors::TEXT_MUTED))
                .child(crate::i18n::t("common.loading")),
        )
}

/// Square 36 px icon-only button used in the control bar (play, mute,
/// settings). `accent=true` forces the active-background to mark
/// "currently engaged" state (e.g. captions on).
pub fn icon_button(
    id: &'static str,
    icon: Option<Arc<Image>>,
    accent: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let mut button = div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(36.0))
        .h(px(36.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(BTN_HOVER)))
        .active(|this| this.bg(rgb(BTN_ACTIVE)))
        .on_click(on_click);

    if accent {
        button = button.bg(rgb(BTN_ACTIVE));
    }
    if let Some(img_handle) = icon {
        button = button.child(
            img(img_handle)
                .w(px(ICON_PX as f32))
                .h(px(ICON_PX as f32)),
        );
    }
    button
}

/// Pill-shaped button: 20 px icon + label. Used for captions / audio /
/// quality pickers where we need to display the current selection.
pub fn icon_label_button(
    id: &'static str,
    icon: Option<Arc<Image>>,
    label: &str,
    accent: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label_color = if accent { ACCENT } else { TEXT_PRIMARY };
    let mut button = div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .h(px(36.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(BTN_HOVER)))
        .active(|this| this.bg(rgb(BTN_ACTIVE)))
        .on_click(on_click);

    if let Some(img_handle) = icon {
        button = button.child(
            img(img_handle)
                .w(px(ICON_PX as f32))
                .h(px(ICON_PX as f32)),
        );
    }
    button.child(
        div()
            .text_xs()
            .text_color(rgb(label_color))
            .font_weight(FontWeight::MEDIUM)
            .child(label.to_string()),
    )
}

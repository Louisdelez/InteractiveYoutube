//! Remote-control modal — opened from the top-bar remote-icon button.
//!
//! Shows the QR code + LAN URL for the smartphone web remote served
//! by `services/remote_server.rs`. Kept separate from the Settings
//! modal so it gets its own dedicated entry point (top-bar button)
//! and because the pairing flow deserves a focused surface — users
//! pull out their phone, scan, done.

use crate::i18n::t;
use gpui::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum RemoteModalEvent {
    Close,
}

impl EventEmitter<RemoteModalEvent> for RemoteModal {}

pub struct RemoteModal {
    /// LAN URL for the remote (e.g. `http://192.168.0.42:4502/?t=…`).
    /// `None` → the embedded server failed to bind at boot, modal
    /// shows a fallback message.
    pub remote_url: Option<String>,
    /// Pre-rasterised QR code. Cached on construction so we don't
    /// re-encode the SVG → PNG on every render.
    pub remote_qr: Option<Arc<Image>>,
}

impl RemoteModal {
    pub fn new(remote_url: Option<String>) -> Self {
        let remote_qr = remote_url.as_deref().and_then(render_qr_png);
        Self { remote_url, remote_qr }
    }
}

/// SVG QR code → rasterised `Arc<gpui::Image>` via resvg + png.
/// Same pattern as `views/icons.rs`. Dark-on-light 240×240.
fn render_qr_png(url: &str) -> Option<Arc<Image>> {
    let svg = crate::services::remote_server::qr_code_svg(url)?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(svg.as_bytes(), &opt).ok()?;
    let size = 240u32;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut png_out = Vec::new();
    let mut encoder = png::Encoder::new(&mut png_out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(pixmap.data()).ok()?;
    drop(writer);
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, png_out)))
}

impl Render for RemoteModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(360.0))
            .bg(rgb(0x18181b))
            .border_1()
            .border_color(rgb(0x2d2d35))
            .rounded(px(8.0))
            .p_4()
            .gap_2()
            // Header : title + close
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
                            .child(t("remote.title")),
                    )
                    .child(
                        div()
                            .id("remote-close")
                            .text_xs()
                            .text_color(rgb(0xaaaaaa))
                            .cursor_pointer()
                            .hover(|this| this.text_color(rgb(0xefeff1)))
                            .child("✕")
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(RemoteModalEvent::Close);
                            })),
                    ),
            )
            // Description
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x9b9b9e))
                    .child(t("remote.description")),
            )
            // QR code or fallback
            .child({
                let body: AnyElement = if let Some(qr) = self.remote_qr.clone() {
                    let url_display = self.remote_url.clone().unwrap_or_default();
                    div()
                        .mt(px(10.0))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            img(qr)
                                .w(px(240.0))
                                .h(px(240.0))
                                .rounded(px(8.0)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x9b9b9e))
                                .child(url_display),
                        )
                        .into_any_element()
                } else {
                    div()
                        .mt(px(10.0))
                        .p_4()
                        .bg(rgb(0x1f1f24))
                        .rounded(px(6.0))
                        .text_xs()
                        .text_color(rgb(0xeab308))
                        .child(t("remote.unavailable"))
                        .into_any_element()
                };
                body
            })
    }
}

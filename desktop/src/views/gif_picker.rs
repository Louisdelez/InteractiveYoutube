use gpui::*;
use std::collections::HashMap;
use std::sync::Arc;

use crate::i18n::t;
use crate::services::api::{self, GifResult};
use std::sync::atomic::{AtomicUsize, Ordering};

static GIF_CONCURRENT: AtomicUsize = AtomicUsize::new(0);
const MAX_GIF_CONCURRENT: usize = 6;

#[derive(Clone, Debug)]
pub struct GifSelected(pub String);

impl EventEmitter<GifSelected> for GifPicker {}

pub struct GifPicker {
    gifs: Vec<GifResult>,
    preview_cache: HashMap<String, Arc<Image>>,
    preview_pending: std::collections::HashSet<String>,
    search_text: String,
    loading: bool,
    search_input: Entity<gpui_component::input::InputState>,
    _subs: Vec<Subscription>,
}

impl GifPicker {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx)
                .placeholder(t("common.search_gif"))
        });

        let input_clone = search_input.clone();
        let sub = cx.subscribe_in(
            &search_input,
            window,
            move |this: &mut Self, _state, ev: &gpui_component::input::InputEvent, _window, cx| {
                if matches!(ev, gpui_component::input::InputEvent::Change) {
                    let q = input_clone.read(cx).value().to_string();
                    this.search_text = q;
                    this.do_search(cx);
                }
            },
        );

        let mut picker = Self {
            gifs: Vec::new(),
            preview_cache: HashMap::new(),
            preview_pending: std::collections::HashSet::new(),
            search_text: String::new(),
            loading: true,
            search_input,
            _subs: vec![sub],
        };
        picker.fetch_trending(cx);
        picker
    }

    fn fetch_trending(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(api::fetch_trending_gifs().ok());
            });
            for _ in 0..80 {
                if let Ok(result) = rx.try_recv() {
                    if let Some(e) = entity.upgrade() {
                        let _ = cx.update_entity(&e, |this, cx| {
                            this.gifs = result.unwrap_or_default();
                            this.loading = false;
                            cx.notify();
                        });
                    }
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
            if let Some(e) = entity.upgrade() {
                let _ = cx.update_entity(&e, |this, cx| {
                    this.loading = false;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn do_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_text.clone();
        if query.trim().is_empty() {
            self.fetch_trending(cx);
            return;
        }
        self.loading = true;
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(api::search_gifs(&query).ok());
            });
            for _ in 0..80 {
                if let Ok(result) = rx.try_recv() {
                    if let Some(e) = entity.upgrade() {
                        let _ = cx.update_entity(&e, |this, cx| {
                            this.gifs = result.unwrap_or_default();
                            this.loading = false;
                            cx.notify();
                        });
                    }
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    }

    fn load_preview(&mut self, url: &str, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        if let Some(cached) = self.preview_cache.get(url) {
            return Some(cached.clone());
        }
        if self.preview_pending.contains(url) {
            return None;
        }
        if GIF_CONCURRENT.load(Ordering::Relaxed) >= MAX_GIF_CONCURRENT {
            return None;
        }
        GIF_CONCURRENT.fetch_add(1, Ordering::Relaxed);
        self.preview_pending.insert(url.to_string());
        let url_owned = url.to_string();
        let entity = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let (tx, rx) = std::sync::mpsc::channel();
            let url_clone = url_owned.clone();
            std::thread::spawn(move || {
                let _ = tx.send(api::fetch_bytes(&url_clone).ok());
            });
            for _ in 0..80 {
                if let Ok(Some(bytes)) = rx.try_recv() {
                    let format = if url_owned.contains(".gif") {
                        ImageFormat::Gif
                    } else if url_owned.contains(".png") {
                        ImageFormat::Png
                    } else {
                        ImageFormat::Jpeg
                    };
                    let image = Arc::new(Image::from_bytes(format, bytes));
                    GIF_CONCURRENT.fetch_sub(1, Ordering::Relaxed);
                    if let Some(e) = entity.upgrade() {
                        let _ = cx.update_entity(&e, |this, cx| {
                            this.preview_pending.remove(&url_owned);
                            this.preview_cache.insert(url_owned.clone(), image);
                            cx.notify();
                        });
                    }
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            // Timeout — release semaphore + clear pending
            GIF_CONCURRENT.fetch_sub(1, Ordering::Relaxed);
            if let Some(e) = entity.upgrade() {
                let _ = cx.update_entity(&e, |this, cx| {
                    this.preview_pending.remove(&url_owned);
                    cx.notify();
                });
            }
        })
        .detach();
        None
    }
}

impl Render for GifPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let urls: Vec<(String, String)> = self.gifs
            .iter()
            .map(|g| (g.gif_url.clone(), g.preview_url.clone()))
            .collect();
        let mut tiles: Vec<(String, Option<Arc<Image>>)> = Vec::new();
        for (gif_url, preview_url) in &urls {
            let preview = self.load_preview(preview_url, cx);
            tiles.push((gif_url.clone(), preview));
        }

        div()
            .w_full()
            .h(px(320.0))
            .bg(rgb(0x1f1f23))
            .border_1()
            .border_color(rgb(0x2d2d30))
            .rounded(px(6.0))
            .shadow_lg()
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(rgb(0x2d2d30))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0x9b59b6))
                            .child("GIF")
                    )
                    .child(
                        div().flex_1().child(
                            gpui_component::input::Input::new(&self.search_input)
                        )
                    )
            )
            .child(
                div()
                    .id("gif-grid")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2()
                    .child(if self.loading {
                        div()
                            .w_full()
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(rgb(0x666666))
                            .child(t("common.loading"))
                            .into_any_element()
                    } else if tiles.is_empty() {
                        div()
                            .w_full()
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(rgb(0x666666))
                            .child(t("common.no_gif_found"))
                            .into_any_element()
                    } else {
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .children(
                                tiles.into_iter().enumerate().map(|(i, (gif_url, preview))| {
                                    let url = gif_url.clone();
                                    let mut tile = div()
                                        .id(("gif", i))
                                        .w(px(95.0))
                                        .h(px(95.0))
                                        .rounded(px(4.0))
                                        .overflow_hidden()
                                        .cursor_pointer()
                                        .bg(rgb(0x2d2d30))
                                        .hover(|this| this.opacity(0.8))
                                        .on_click(cx.listener(move |_this, _, _, cx| {
                                            cx.emit(GifSelected(url.clone()));
                                        }));
                                    if let Some(img_handle) = preview {
                                        tile = tile.child(
                                            img(img_handle)
                                                .id(ElementId::Name(format!("gif-preview-{i}").into()))
                                                .w(px(95.0))
                                                .h(px(95.0))
                                                .object_fit(ObjectFit::Cover)
                                        );
                                    }
                                    tile
                                }).collect::<Vec<_>>()
                            )
                            .into_any_element()
                    })
            )
            .child(
                div()
                    .px_2()
                    .py(px(2.0))
                    .border_t_1()
                    .border_color(rgb(0x2d2d30))
                    .text_color(rgb(0x444444))
                    .child(
                        div().text_size(px(9.0)).child("Powered by Tenor")
                    )
            )
    }
}

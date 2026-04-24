use gpui::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use crate::i18n::t;
use crate::models::message::ChatMessage;
use crate::services::emoji_data;
use crate::views::emoji_input::{EmojiInput, EmojiInputSubmit};
use crate::views::gif_picker::{GifPicker, GifSelected};

const MAX_MESSAGES: usize = 500;
const RENDER_WINDOW: usize = 60;

/// Event emitted when user sends a chat message (Enter pressed).
#[derive(Clone, Debug)]
pub struct ChatSend {
    pub text: String,
}

impl EventEmitter<ChatSend> for ChatView {}

/// Which tab is active in the unified emoji/GIF picker.
#[derive(Clone, Copy, PartialEq)]
enum PickerTab {
    Emoji,
    Gif,
    Stickers,
}

pub struct ChatView {
    pub messages: VecDeque<ChatMessage>,
    pub viewer_count: usize,
    emoji_input: Entity<EmojiInput>,
    /// The unified picker is open (true) or closed (false).
    picker_open: bool,
    /// Which tab is active inside the picker.
    picker_tab: PickerTab,
    gif_picker: Option<Entity<GifPicker>>,
    emoji_category: usize,
    emoji_cache: HashMap<String, Arc<Image>>,
    /// Cache segment_text results per unique message to avoid 4936-pattern
    /// scan on every render frame.
    segment_cache: HashMap<String, Vec<(bool, String)>>,
    gif_cache: HashMap<String, Arc<Image>>,
    gif_pending: std::collections::HashSet<String>,
    sticker_cache: HashMap<String, Arc<Image>>,
    sticker_list: Option<Vec<String>>,
    messages_scroll: ScrollHandle,
    /// True = auto-scroll to bottom on new messages. Disabled when user
    /// scrolls up manually, re-enabled on new message arrival.
    auto_scroll: bool,
    icons: crate::views::icons::IconCache,
    #[allow(dead_code)]
    _subs: Vec<Subscription>,
}

/// Path to the Apple emoji PNG for the given codepoint string.
fn emoji_png_path(u: &str) -> String {
    format!(
        "{}/assets/emoji-png/{}.png",
        env!("CARGO_MANIFEST_DIR"),
        u
    )
}

impl ChatView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let emoji_input = cx.new(|cx| EmojiInput::new(cx));

        let sub = cx.subscribe(
            &emoji_input,
            move |_this: &mut Self, _input, ev: &EmojiInputSubmit, cx| {
                cx.emit(ChatSend { text: ev.0.clone() });
            },
        );

        Self {
            messages: VecDeque::with_capacity(MAX_MESSAGES + 1),
            viewer_count: 0,
            emoji_input,
            picker_open: false,
            picker_tab: PickerTab::Emoji,
            gif_picker: None,
            emoji_category: 0,
            emoji_cache: HashMap::new(),
            segment_cache: HashMap::new(),
            gif_cache: HashMap::new(),
            gif_pending: std::collections::HashSet::new(),
            sticker_cache: HashMap::new(),
            sticker_list: None,
            messages_scroll: ScrollHandle::new(),
            auto_scroll: true,
            icons: crate::views::icons::IconCache::new(),
            _subs: vec![sub],
        }
    }

    fn scroll_to_bottom(&mut self, _cx: &mut Context<Self>) {
        if self.auto_scroll {
            self.messages_scroll.scroll_to_bottom();
        }
    }

    /// Replace the entire history (called on chat:history from server).
    pub fn replace_messages(&mut self, history: Vec<(String, String, String, String)>, cx: &mut Context<Self>) {
        // Don't wipe existing messages with an empty history —
        // this happens on reconnect when the server sends
        // chat:history for its random default room (which is
        // often empty). Our ChatChannelChanged follows shortly
        // with the real history.
        if history.is_empty() && !self.messages.is_empty() {
            return;
        }
        self.messages.clear();
        for (username, text, color, time) in history.into_iter().rev().take(MAX_MESSAGES).rev() {
            self.messages.push_back(ChatMessage {
                id: String::new(),
                username,
                text,
                color,
                registered: false,
                time,
            });
        }
        self.scroll_to_bottom(cx);
    }

    pub fn push_message(&mut self, username: String, text: String, color: String, time: String, cx: &mut Context<Self>) {
        self.messages.push_back(ChatMessage {
            id: String::new(),
            username,
            text,
            color,
            registered: false,
            time,
        });
        while self.messages.len() > MAX_MESSAGES {
            self.messages.pop_front();
        }
        self.scroll_to_bottom(cx);
    }

    pub fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.messages.clear();
        self.scroll_to_bottom(cx);
    }

    pub fn set_viewer_count(&mut self, count: usize) {
        self.viewer_count = count;
    }
}

impl ChatView {
    fn append_to_input(&self, emoji: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.emoji_input.update(cx, |input, cx| {
            input.append(emoji);
            cx.notify();
        });
    }

    /// Load (and cache) the Apple PNG for the given emoji code. Returns None
    /// if the file is missing on disk.
    fn emoji_image(&mut self, u: &str) -> Option<Arc<Image>> {
        if let Some(img) = self.emoji_cache.get(u) {
            return Some(img.clone());
        }
        let bytes = std::fs::read(emoji_png_path(u)).ok()?;
        let img = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
        self.emoji_cache.insert(u.to_string(), img.clone());
        Some(img)
    }

    fn sticker_dir() -> String {
        format!("{}/assets/stickers", env!("CARGO_MANIFEST_DIR"))
    }

    fn load_sticker(&mut self, name: &str) -> Option<Arc<Image>> {
        if let Some(cached) = self.sticker_cache.get(name) {
            return Some(cached.clone());
        }
        let dir = Self::sticker_dir();
        // Try .png then .gif
        let path = std::path::Path::new(&dir).join(name);
        let bytes = std::fs::read(&path).ok()?;
        let format = if name.ends_with(".gif") {
            ImageFormat::Gif
        } else {
            ImageFormat::Png
        };
        let image = Arc::new(Image::from_bytes(format, bytes));
        self.sticker_cache.insert(name.to_string(), image.clone());
        Some(image)
    }

    fn list_stickers() -> Vec<String> {
        let dir = Self::sticker_dir();
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".png") || name.ends_with(".gif") {
                    names.push(name);
                }
            }
        }
        names.sort();
        names
    }

    /// Render text with emoji characters replaced by Apple PNG images
    /// inline. `size` is the emoji image size in px.
    fn render_rich_text(&mut self, text: &str, emoji_size: f32) -> Div {
        let cached = self.segment_cache.get(text).cloned();
        let raw_segments;
        let segments_ref: &[(bool, String)] = if let Some(ref c) = cached {
            c
        } else {
            let segs = emoji_data::segment_text(text);
            let flat: Vec<(bool, String)> = segs.into_iter().map(|s| match s {
                emoji_data::TextSegment::Text(t) => (false, t),
                emoji_data::TextSegment::Emoji(u) => (true, u),
            }).collect();
            if self.segment_cache.len() > 500 { self.segment_cache.clear(); }
            self.segment_cache.insert(text.to_string(), flat.clone());
            raw_segments = flat;
            &raw_segments
        };
        let segments: Vec<emoji_data::TextSegment> = segments_ref.iter().map(|(is_emoji, s)| {
            if *is_emoji { emoji_data::TextSegment::Emoji(s.clone()) }
            else { emoji_data::TextSegment::Text(s.clone()) }
        }).collect();
        let has_emoji = segments.iter().any(|s| matches!(s, emoji_data::TextSegment::Emoji(_)));
        if !has_emoji {
            return div().text_color(rgb(0xefeff1)).child(text.to_string());
        }
        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .text_color(rgb(0xefeff1));
        for seg in segments {
            match seg {
                emoji_data::TextSegment::Text(t) => {
                    row = row.child(t);
                }
                emoji_data::TextSegment::Emoji(u) => {
                    if let Some(img_data) = self.emoji_image(&u) {
                        row = row.child(
                            img(img_data)
                                .size(px(emoji_size))
                                .flex_shrink_0()
                        );
                    } else {
                        row = row.child(emoji_data::unicode_from_u(&u));
                    }
                }
            }
        }
        row
    }

    /// Render message body in chat (16 px emojis). GIF messages
    /// (`[gif:url]`) are rendered as inline animated images.
    fn render_message_body(&mut self, text: &str, cx: &mut Context<Self>) -> Div {
        // Sticker messages: [sticker:filename]
        if let Some(name) = text.strip_prefix("[sticker:")
            .and_then(|s| s.strip_suffix(']'))
        {
            let name = name.to_string();
            if let Some(img_data) = self.load_sticker(&name) {
                let id_str: SharedString = format!("sticker-{name}").into();
                return div().child(
                    img(img_data)
                        .id(ElementId::Name(id_str))
                        .w(px(128.0))
                        .rounded(px(4.0))
                );
            }
            return div().text_color(rgb(0x9b59b6)).text_xs().child(format!("[sticker:{name}]"));
        }
        // GIF messages: [gif:url]
        if let Some(url) = text.strip_prefix("[gif:")
            .and_then(|s| s.strip_suffix(']'))
        {
            let url = url.to_string();
            if let Some(cached) = self.gif_cache.get(&url) {
                let id_str: SharedString = format!("chat-gif-{}", &url[url.len().saturating_sub(20)..]).into();
                return div().child(
                    img(cached.clone())
                        .id(ElementId::Name(id_str))
                        .w(px(200.0))
                        .rounded(px(4.0))
                );
            }
            if self.gif_pending.contains(&url) {
                return div()
                    .w(px(200.0))
                    .h(px(100.0))
                    .rounded(px(4.0))
                    .bg(rgb(0x2d2d30))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(rgb(0x666666))
                    .child(t("chat.gif_loading"));
            }
            self.gif_pending.insert(url.clone());
            let url_clone = url.clone();
            let entity = cx.entity().downgrade();
            cx.spawn(async move |_, cx| {
                let (tx, rx) = std::sync::mpsc::channel();
                let url_for_thread = url_clone.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::services::api::fetch_bytes(&url_for_thread).ok());
                });
                for _ in 0..80 {
                    if let Ok(Some(bytes)) = rx.try_recv() {
                        let format = if url_clone.contains(".gif") {
                            ImageFormat::Gif
                        } else {
                            ImageFormat::Jpeg
                        };
                        let image = Arc::new(Image::from_bytes(format, bytes));
                        if let Some(e) = entity.upgrade() {
                            let _ = cx.update_entity(&e, |this, cx| {
                                this.gif_pending.remove(&url_clone);
                                this.gif_cache.insert(url_clone.clone(), image);
                                cx.notify();
                            });
                        }
                        return;
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                }
                // Timeout — clear pending so it can retry later
                if let Some(e) = entity.upgrade() {
                    let _ = cx.update_entity(&e, |this, _cx| {
                        this.gif_pending.remove(&url_clone);
                    });
                }
            })
            .detach();
            return div()
                .w(px(200.0))
                .h(px(100.0))
                .rounded(px(4.0))
                .bg(rgb(0x2d2d30))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(rgb(0x666666))
                .child(t("chat.gif_loading"));
        }
        self.render_rich_text(text, 16.0)
    }

}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(340.0))
            .h_full()
            .bg(rgb(0x18181b))
            .border_l_1()
            .border_color(rgb(0x2d2d30))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x2d2d30))
                    .child(
                        div().text_sm().font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xefeff1))
                            .child(t("chat.title"))
                    )
                    .child({
                        // Match the web ViewerCount: eye + bold count, both
                        // light violet (#BF94FF — the same as the web CSS).
                        let eye = self.icons.get(
                            crate::views::icons::IconName::Eye,
                            14,
                            0xbf94ff,
                        );
                        let mut row = div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_color(rgb(0xbf94ff))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD);
                        if let Some(icon) = eye {
                            row = row.child(img(icon).w(px(14.0)).h(px(14.0)));
                        }
                        row.child(format!("{}", self.viewer_count))
                    })
            )
            .child(
                if self.messages.is_empty() {
                    div()
                        .id("chat-messages")
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(rgb(0x666666))
                        .child(t("chat.empty"))
                } else {
                    if self.auto_scroll {
                        self.messages_scroll.scroll_to_bottom();
                    }
                    div()
                        .id("chat-messages")
                        .track_scroll(&self.messages_scroll)
                        .flex()
                        .flex_col()
                        .flex_1()
                        .overflow_y_scroll()
                        .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, _cx| {
                            let dy = ev.delta.pixel_delta(px(20.0)).y;
                            if dy > px(0.0) {
                                this.auto_scroll = false;
                            }
                        }))
                        .gap(px(2.0))
                        .px_2()
                        .py_1()
                        .children({
                            // Render only the tail of the buffer (oldest
                            // first so the scroll anchor at the bottom
                            // stays on the newest message). `skip()` on
                            // a VecDeque iterator walks indices in
                            // contiguous memory — no intermediate Vec.
                            let start = self.messages.len().saturating_sub(RENDER_WINDOW);
                            {
                            // Clone only text (needed for &mut self in
                            // render_message_body). Other fields are
                            // borrowed via index after bodies are built.
                            let texts: Vec<String> = self.messages
                                .iter()
                                .skip(start)
                                .map(|msg| msg.text.clone())
                                .collect();
                            let rendered: Vec<_> = texts.iter().map(|text| {
                                self.render_message_body(text, cx)
                            }).collect();
                            let meta: Vec<_> = self.messages
                                .iter()
                                .skip(start)
                                .map(|msg| (msg.time.clone(), msg.username.clone(), parse_hex_color(&msg.color).unwrap_or(0xaaaaaa)))
                                .collect();
                            meta.into_iter().zip(rendered).map(|((time, username, color), body)| {
                                div()
                                    .px_2()
                                    .py(px(2.0))
                                    .text_xs()
                                    .child(
                                        div()
                                            .flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_color(rgb(0x666666))
                                                    .child(time)
                                            )
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(rgb(color))
                                                    .child(username)
                                            )
                                            .child(body)
                                    )
                            })
                            }
                        })
                }
            )
            .child(if !self.auto_scroll {
                div()
                    .flex()
                    .justify_center()
                    .py(px(4.0))
                    .border_t_1()
                    .border_color(rgb(0x2d2d30))
                    .child(
                        div()
                            .id("scroll-to-bottom-btn")
                            .px_3()
                            .py(px(3.0))
                            .rounded(px(12.0))
                            .bg(rgb(0x9b59b6))
                            .text_xs()
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0xac6dc7)))
                            .child(t("chat.new_messages"))
                            .on_click(cx.listener(|this: &mut ChatView, _, _, cx| {
                                this.auto_scroll = true;
                                this.messages_scroll.scroll_to_bottom();
                                cx.notify();
                            }))
                    )
                    .into_any_element()
            } else {
                div().into_any_element()
            })
            .child({
                let mut footer = div()
                    .relative()
                    .px_2()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(0x2d2d30));

                // ── Unified picker popup (Emoji + GIF tabs) ──────────
                if self.picker_open {
                    // Ensure GIF picker entity exists
                    if self.gif_picker.is_none() {
                        let gp = cx.new(|cx| GifPicker::new(window, cx));
                        let sub = cx.subscribe(
                            &gp,
                            move |this: &mut ChatView, _picker, ev: &GifSelected, cx| {
                                let msg = format!("[gif:{}]", ev.0);
                                cx.emit(ChatSend { text: msg });
                                this.picker_open = false;
                                cx.notify();
                            },
                        );
                        self._subs.push(sub);
                        self.gif_picker = Some(gp);
                    }

                    // ── Top-level tabs: Emoji | GIF ──
                    let cur_tab = self.picker_tab;
                    let make_tab = |id: &'static str, label: &'static str, tab: PickerTab, cx: &mut Context<ChatView>| {
                        let active = cur_tab == tab;
                        div()
                            .id(id)
                            .px_2()
                            .py(px(4.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if active { rgb(0xefeff1) } else { rgb(0x666666) })
                            .bg(if active { rgb(0x2d2d30) } else { rgb(0x1f1f23) })
                            .hover(|this| this.bg(rgb(0x2d2d30)))
                            .child(label)
                            .on_click(cx.listener(move |this: &mut ChatView, _, _, cx| {
                                this.picker_tab = tab;
                                cx.notify();
                            }))
                    };
                    let tab_bar = div()
                        .flex()
                        .items_center()
                        .px_2()
                        .py_1()
                        .gap_1()
                        .border_b_1()
                        .border_color(rgb(0x2d2d30))
                        .child(make_tab("tab-emoji", "Emoji", PickerTab::Emoji, cx))
                        .child(make_tab("tab-gif", "GIF", PickerTab::Gif, cx))
                        .child(make_tab("tab-stickers", "Stickers", PickerTab::Stickers, cx));

                    // ── Content area ──
                    let content: AnyElement = if cur_tab == PickerTab::Emoji {
                        // Emoji sub-tabs + grid
                        let cats = emoji_data::categories();
                        let active_cat_idx = self.emoji_category.min(cats.len().saturating_sub(1));
                        let mut tab_icons: Vec<Option<Arc<Image>>> = Vec::with_capacity(cats.len());
                        for c in cats {
                            let icon = c.emojis.first().map(|e| e.u.clone());
                            let im = icon.as_deref().and_then(|u| self.emoji_image(u));
                            tab_icons.push(im);
                        }
                        let active = &cats[active_cat_idx];
                        let mut tile_data: Vec<(String, Arc<Image>)> =
                            Vec::with_capacity(active.emojis.len());
                        for e in &active.emojis {
                            if let Some(im) = self.emoji_image(&e.u) {
                                tile_data.push((e.u.clone(), im));
                            }
                        }
                        let emoji_sub_tabs = cats.iter().enumerate().map(|(i, _)| {
                            let is_active = i == active_cat_idx;
                            let icon_img = tab_icons[i].clone();
                            let mut tab = div()
                                .id(("emoji-tab", i))
                                .w(px(24.0))
                                .h(px(24.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|this| this.bg(rgb(0x2d2d30)))
                                .on_click(cx.listener(move |this: &mut ChatView, _, _, cx| {
                                    this.emoji_category = i;
                                    cx.notify();
                                }));
                            if is_active { tab = tab.bg(rgb(0x2d2d30)); }
                            if let Some(h) = icon_img {
                                tab.child(img(h).w(px(16.0)).h(px(16.0)))
                            } else {
                                tab.child(div().text_size(px(9.0)).text_color(rgb(0xaaaaaa)).child("·"))
                            }
                        }).collect::<Vec<_>>();

                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(2.0))
                                    .px_2()
                                    .py(px(2.0))
                                    .border_b_1()
                                    .border_color(rgb(0x2d2d30))
                                    .children(emoji_sub_tabs)
                            )
                            .child(
                                div()
                                    .id("emoji-grid")
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .p_2()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_1()
                                            .children(tile_data.into_iter().enumerate().map(|(i, (u, img_handle))| {
                                                let unicode = emoji_data::unicode_from_u(&u);
                                                div()
                                                    .id(("emoji", i))
                                                    .w(px(28.0))
                                                    .h(px(28.0))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(px(4.0))
                                                    .cursor_pointer()
                                                    .hover(|this| this.bg(rgb(0x2d2d30)))
                                                    .on_click(cx.listener(move |this: &mut ChatView, _, window, cx| {
                                                        this.append_to_input(&unicode, window, cx);
                                                        this.picker_open = false;
                                                        cx.notify();
                                                    }))
                                                    .child(img(img_handle).w(px(22.0)).h(px(22.0)))
                                            }).collect::<Vec<_>>())
                                    )
                            )
                            .into_any_element()
                    } else if cur_tab == PickerTab::Gif {
                        // GIF tab — delegate to GifPicker entity
                        if let Some(ref picker) = self.gif_picker {
                            div().flex_1().child(picker.clone()).into_any_element()
                        } else {
                            div().flex_1().into_any_element()
                        }
                    } else {
                        // Stickers tab (cached — avoids fs::read_dir every frame)
                        if self.sticker_list.is_none() {
                            self.sticker_list = Some(Self::list_stickers());
                        }
                        let sticker_names = self.sticker_list.clone().unwrap_or_default();
                        let mut sticker_tiles: Vec<(String, Arc<Image>)> = Vec::new();
                        for name in &sticker_names {
                            if let Some(im) = self.load_sticker(name) {
                                sticker_tiles.push((name.clone(), im));
                            }
                        }
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .border_b_1()
                                    .border_color(rgb(0x2d2d30))
                                    .text_xs()
                                    .text_color(rgb(0x666666))
                                    .child(format!("{} sticker(s)", sticker_tiles.len()))
                            )
                            .child(
                                div()
                                    .id("sticker-grid")
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .p_2()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_wrap()
                                            .gap_2()
                                            .children(sticker_tiles.into_iter().enumerate().map(|(i, (name, img_handle))| {
                                                let name_clone = name.clone();
                                                div()
                                                    .id(("sticker", i))
                                                    .w(px(64.0))
                                                    .h(px(64.0))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(px(6.0))
                                                    .cursor_pointer()
                                                    .bg(rgb(0x18181b))
                                                    .hover(|this| this.bg(rgb(0x2d2d30)))
                                                    .on_click(cx.listener(move |this: &mut ChatView, _, _, cx| {
                                                        let msg = format!("[sticker:{}]", name_clone);
                                                        cx.emit(ChatSend { text: msg });
                                                        this.picker_open = false;
                                                        cx.notify();
                                                    }))
                                                    .child(
                                                        img(img_handle)
                                                            .id(ElementId::Name(format!("sticker-preview-{i}").into()))
                                                            .w(px(56.0))
                                                            .h(px(56.0))
                                                            .object_fit(ObjectFit::Contain)
                                                    )
                                            }).collect::<Vec<_>>())
                                    )
                            )
                            .into_any_element()
                    };

                    footer = footer.child(
                        div()
                            .absolute()
                            .bottom(px(48.0))
                            .left_2()
                            .right_2()
                            .h(px(360.0))
                            .bg(rgb(0x1f1f23))
                            .border_1()
                            .border_color(rgb(0x2d2d30))
                            .rounded(px(6.0))
                            .shadow_lg()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .on_scroll_wheel(|_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(tab_bar)
                            .child(content)
                    );
                }

                // ── Single icon button + input + send ──────────────
                let icon_color = if self.picker_open { 0x9b59b6 } else { 0xaaaaaa };
                let icon_handle = self.icons.get(
                    crate::views::icons::IconName::SmilePlus,
                    20,
                    icon_color,
                );
                footer.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child({
                            let mut btn = div()
                                .id("picker-toggle")
                                .w(px(28.0))
                                .h(px(28.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|this| this.bg(rgb(0x2d2d30)))
                                .on_click(cx.listener(|this: &mut ChatView, _, _, cx| {
                                    this.picker_open = !this.picker_open;
                                    if !this.picker_open {
                                        this.gif_picker = None;
                                    }
                                    cx.notify();
                                }));
                            if let Some(h) = icon_handle {
                                btn = btn.child(img(h).size(px(20.0)));
                            } else {
                                btn = btn.child(div().text_xs().text_color(rgb(icon_color)).child("+"));
                            }
                            btn
                        })
                        .child(
                            div().flex_1().child(self.emoji_input.clone())
                        )
                        .child({
                            let input_handle = self.emoji_input.clone();
                            div()
                                .id("chat-send-btn")
                                .px_3()
                                .h(px(28.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .bg(rgb(0x9b59b6))
                                .text_color(rgb(0xffffff))
                                .cursor_pointer()
                                .hover(|this| this.bg(rgb(0xac6dc7)))
                                .child(crate::i18n::t("chat.send"))
                                .on_click(cx.listener(move |_this: &mut ChatView, _, _window, cx| {
                                    let text = input_handle.read(cx).text().trim().to_string();
                                    if text.is_empty() { return; }
                                    cx.emit(ChatSend { text });
                                    input_handle.update(cx, |s, cx| {
                                        s.clear();
                                        cx.notify();
                                    });
                                }))
                        })
                )
            })
    }
}

fn parse_hex_color(s: &str) -> Option<u32> {
    let s = s.trim_start_matches('#');
    u32::from_str_radix(s, 16).ok()
}

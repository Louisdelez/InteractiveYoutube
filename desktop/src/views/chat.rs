use gpui::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use crate::models::message::ChatMessage;
use crate::services::emoji_data;
use crate::views::emoji_input::{EmojiInput, EmojiInputSubmit};

const MAX_MESSAGES: usize = 500;
const RENDER_WINDOW: usize = 60;

/// Event emitted when user sends a chat message (Enter pressed).
#[derive(Clone, Debug)]
pub struct ChatSend {
    pub text: String,
}

impl EventEmitter<ChatSend> for ChatView {}

pub struct ChatView {
    pub messages: VecDeque<ChatMessage>,
    pub viewer_count: usize,
    emoji_input: Entity<EmojiInput>,
    show_emoji: bool,
    emoji_category: usize,
    emoji_cache: HashMap<String, Arc<Image>>,
    messages_scroll: ScrollHandle,
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
            show_emoji: false,
            emoji_category: 0,
            emoji_cache: HashMap::new(),
            messages_scroll: ScrollHandle::new(),
            icons: crate::views::icons::IconCache::new(),
            _subs: vec![sub],
        }
    }

    /// Snap the message list to the latest message. Called after every
    /// new message (incoming or outgoing) — Twitch-style auto-follow.
    fn scroll_to_bottom(&self) {
        let visible = self.messages.len().min(RENDER_WINDOW);
        if visible > 0 {
            self.messages_scroll.scroll_to_item(visible - 1);
        }
    }

    /// Replace the entire history (called on chat:history from server).
    pub fn replace_messages(&mut self, history: Vec<(String, String, String, String)>) {
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
        self.scroll_to_bottom();
    }

    pub fn push_message(&mut self, username: String, text: String, color: String, time: String) {
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
        self.scroll_to_bottom();
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

    /// Render text with emoji characters replaced by Apple PNG images
    /// inline. `size` is the emoji image size in px.
    fn render_rich_text(&mut self, text: &str, emoji_size: f32) -> Div {
        let segments = emoji_data::segment_text(text);
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

    /// Render message body in chat (16 px emojis).
    fn render_message_body(&mut self, text: &str) -> Div {
        self.render_rich_text(text, 16.0)
    }

}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .child("Chat en direct")
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
                        .child("Pas encore de messages.")
                } else {
                    // Messages rendered as DIRECT children of the scrollable
                    // div — required so `track_scroll(&messages_scroll)` can
                    // locate per-message child bounds and `scroll_to_item`
                    // can target the last one. With a wrapper child the
                    // handle would only see "1 child" and never scroll.
                    div()
                        .id("chat-messages")
                        .track_scroll(&self.messages_scroll)
                        .flex()
                        .flex_col()
                        .flex_1()
                        .overflow_y_scroll()
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
                            // Collect message data first (immutable borrow
                            // on self.messages), then render bodies
                            // separately (mutable borrow for emoji cache).
                            let msg_data: Vec<_> = self.messages
                                .iter()
                                .skip(start)
                                .map(|msg| (msg.time.clone(), msg.username.clone(), msg.color.clone(), msg.text.clone()))
                                .collect();
                            let rendered: Vec<_> = msg_data.iter().map(|(_, _, _, text)| {
                                self.render_message_body(text)
                            }).collect();
                            msg_data.into_iter().zip(rendered).map(|((time, username, color, _), body)| {
                                let color = parse_hex_color(&color).unwrap_or(0xaaaaaa);
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
            .child({
                let show_emoji = self.show_emoji;
                let mut footer = div()
                    .relative()
                    .px_2()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(0x2d2d30));

                if show_emoji {
                    let cats = emoji_data::categories();
                    let active_cat_idx = self.emoji_category.min(cats.len().saturating_sub(1));
                    // Pre-load tab icons (first emoji of each category) so they
                    // appear immediately on first open instead of after a click.
                    let mut tab_icons: Vec<Option<Arc<Image>>> = Vec::with_capacity(cats.len());
                    for c in cats {
                        let icon = c.emojis.first().map(|e| e.u.clone());
                        let img = icon.as_deref().and_then(|u| self.emoji_image(u));
                        tab_icons.push(img);
                    }
                    // Pre-resolve images for the visible category so we can move
                    // owned handles into the per-emoji closures without needing
                    // `&mut self` from inside the children iterator.
                    let active = &cats[active_cat_idx];
                    let mut tile_data: Vec<(String, String, Arc<Image>)> =
                        Vec::with_capacity(active.emojis.len());
                    for e in &active.emojis {
                        // Skip emojis with no Apple PNG (Apple omits a handful of
                        // combining symbols like ♀️ ♂️ ⚕️). Avoids ugly black-on-
                        // dark fallback glyphs in the picker.
                        if let Some(img) = self.emoji_image(&e.u) {
                            tile_data.push((e.u.clone(), e.name.clone(), img));
                        }
                    }

                    let tabs = cats.iter().enumerate().map(|(i, _c)| {
                        let is_active = i == active_cat_idx;
                        let icon_img = tab_icons[i].clone();
                        let mut tab = div()
                            .id(("emoji-tab", i))
                            .w(px(28.0))
                            .h(px(28.0))
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
                        if is_active {
                            tab = tab.bg(rgb(0x2d2d30));
                        }
                        if let Some(h) = icon_img {
                            tab.child(img(h).w(px(20.0)).h(px(20.0)))
                        } else {
                            tab.child(div().text_xs().text_color(rgb(0xaaaaaa)).child("·"))
                        }
                    }).collect::<Vec<_>>();

                    footer = footer.child(
                        div()
                            .absolute()
                            .bottom(px(48.0))
                            .left_2()
                            .right_2()
                            .h(px(320.0))
                            .bg(rgb(0x1f1f23))
                            .border_1()
                            .border_color(rgb(0x2d2d30))
                            .rounded(px(6.0))
                            .shadow_lg()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            // Category tabs
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .py_1()
                                    .border_b_1()
                                    .border_color(rgb(0x2d2d30))
                                    .children(tabs)
                            )
                            // Scrollable emoji grid
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
                                            .children(tile_data.into_iter().enumerate().map(|(i, (u, _name, img_handle))| {
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
                                                        this.show_emoji = false;
                                                        cx.notify();
                                                    }))
                                                    .child(img(img_handle).w(px(22.0)).h(px(22.0)))
                                            }).collect::<Vec<_>>())
                                    )
                            )
                    );
                }

                footer.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .id("emoji-toggle")
                                .w(px(28.0))
                                .h(px(28.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(|this| this.bg(rgb(0x2d2d30)))
                                .child({
                                    if let Some(smiley) = self.emoji_image("1f60a") {
                                        img(smiley).size(px(20.0)).into_any_element()
                                    } else {
                                        div().text_size(px(16.0)).text_color(rgb(0xaaaaaa)).child("😊").into_any_element()
                                    }
                                })
                                .on_click(cx.listener(|this: &mut ChatView, _, _, cx| {
                                    this.show_emoji = !this.show_emoji;
                                    cx.notify();
                                }))
                        )
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
                                .child("Chat")
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

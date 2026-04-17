//! Custom chat input that renders emoji as Apple PNG images inline.
//! Uses GPUI's EntityInputHandler for native keyboard/IME/paste support
//! (same mechanism Zed uses for its editor buffers).

use gpui::*;
use std::ops::Range;
use std::sync::Arc;
use std::collections::HashMap;

use crate::services::emoji_data;

const EMOJI_PX: f32 = 14.0;
const CURSOR_BLINK_MS: u64 = 530;

pub struct EmojiInput {
    text: String,
    cursor: usize,
    focus: FocusHandle,
    blink_visible: bool,
    emoji_cache: HashMap<String, Arc<Image>>,
}

#[derive(Clone, Debug)]
pub struct EmojiInputSubmit(pub String);

impl EventEmitter<EmojiInputSubmit> for EmojiInput {}

fn emoji_png_path(u: &str) -> String {
    format!("{}/assets/emoji-png/{}.png", env!("CARGO_MANIFEST_DIR"), u)
}

impl EmojiInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();

        // Blink cursor timer
        cx.spawn(async move |entity, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(CURSOR_BLINK_MS))
                    .await;
                let Some(e) = entity.upgrade() else { break };
                let _ = cx.update_entity(&e, |this, cx| {
                    this.blink_visible = !this.blink_visible;
                    cx.notify();
                });
            }
        })
        .detach();

        Self {
            text: String::new(),
            cursor: 0,
            focus,
            blink_visible: true,
            emoji_cache: HashMap::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn append(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    fn emoji_image(&mut self, u: &str) -> Option<Arc<Image>> {
        if let Some(img) = self.emoji_cache.get(u) {
            return Some(img.clone());
        }
        let bytes = std::fs::read(emoji_png_path(u)).ok()?;
        let img = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
        self.emoji_cache.insert(u.to_string(), img.clone());
        Some(img)
    }

    fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    fn cursor_char_offset(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }

    fn byte_offset_from_char(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    /// Compute byte-offset boundaries for each visual segment (text
    /// chars individually, emoji as one atomic unit). Returns Vec of
    /// (start_byte, end_byte).
    fn visual_boundaries(&self) -> Vec<(usize, usize)> {
        let segments = emoji_data::segment_text(&self.text);
        let mut bounds = Vec::new();
        let mut byte_off = 0;
        for seg in &segments {
            match seg {
                emoji_data::TextSegment::Text(t) => {
                    for ch in t.chars() {
                        let end = byte_off + ch.len_utf8();
                        bounds.push((byte_off, end));
                        byte_off = end;
                    }
                }
                emoji_data::TextSegment::Emoji(u) => {
                    let unicode = emoji_data::unicode_from_u(u);
                    let end = byte_off + unicode.len();
                    bounds.push((byte_off, end));
                    byte_off = end;
                }
            }
        }
        bounds
    }

    fn move_cursor_left(&mut self) {
        if self.cursor == 0 { return; }
        let bounds = self.visual_boundaries();
        for &(start, end) in bounds.iter().rev() {
            if end <= self.cursor {
                self.cursor = start;
                break;
            }
            if start < self.cursor {
                self.cursor = start;
                break;
            }
        }
        self.blink_visible = true;
    }

    fn move_cursor_right(&mut self) {
        if self.cursor >= self.text.len() { return; }
        let bounds = self.visual_boundaries();
        for &(_start, end) in &bounds {
            if end > self.cursor {
                self.cursor = end;
                break;
            }
        }
        self.blink_visible = true;
    }

    fn delete_backward(&mut self) {
        if self.cursor == 0 { return; }
        let bounds = self.visual_boundaries();
        for &(start, end) in bounds.iter().rev() {
            if end <= self.cursor {
                self.text.drain(start..end);
                self.cursor = start;
                break;
            }
            if start < self.cursor {
                self.text.drain(start..end);
                self.cursor = start;
                break;
            }
        }
        self.blink_visible = true;
    }

    fn delete_forward(&mut self) {
        if self.cursor >= self.text.len() { return; }
        let bounds = self.visual_boundaries();
        for &(start, end) in &bounds {
            if start >= self.cursor {
                self.text.drain(start..end);
                break;
            }
        }
        self.blink_visible = true;
    }

    // UTF-16 helpers for EntityInputHandler
    fn utf16_len(&self) -> usize {
        self.text.encode_utf16().count()
    }

    fn byte_to_utf16(&self, byte_offset: usize) -> usize {
        self.text[..byte_offset].encode_utf16().count()
    }

    fn utf16_to_byte(&self, utf16_offset: usize) -> usize {
        let mut utf16_count = 0;
        for (i, c) in self.text.char_indices() {
            if utf16_count >= utf16_offset {
                return i;
            }
            utf16_count += c.len_utf16();
        }
        self.text.len()
    }
}

impl EntityInputHandler for EmojiInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let start = self.utf16_to_byte(range_utf16.start);
        let end = self.utf16_to_byte(range_utf16.end);
        let start = start.min(self.text.len());
        let end = end.min(self.text.len());
        adjusted_range.replace(
            self.byte_to_utf16(start)..self.byte_to_utf16(end),
        );
        Some(self.text[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let pos = self.byte_to_utf16(self.cursor);
        Some(UTF16Selection {
            range: pos..pos,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = if let Some(r) = range_utf16 {
            let start = self.utf16_to_byte(r.start);
            let end = self.utf16_to_byte(r.end);
            start..end
        } else {
            self.cursor..self.cursor
        };
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len());
        self.text.replace_range(start..end, new_text);
        self.cursor = start + new_text.len();
        self.blink_visible = true;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range_utf16, new_text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for EmojiInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.is_focused(window);
        let cursor_byte = self.cursor;

        // Segment text for emoji rendering
        let segments = emoji_data::segment_text(&self.text);

        // Build children: text spans + emoji images + cursor
        let mut children: Vec<AnyElement> = Vec::new();
        let mut byte_off = 0usize;
        let show_cursor = focused && self.blink_visible;

        let cursor_div = || -> AnyElement {
            div()
                .w(px(1.5))
                .h(px(14.0))
                .bg(rgb(0xefeff1))
                .flex_shrink_0()
                .into_any_element()
        };

        for seg in &segments {
            match seg {
                emoji_data::TextSegment::Text(t) => {
                    let seg_end = byte_off + t.len();
                    if show_cursor && cursor_byte >= byte_off && cursor_byte < seg_end {
                        let split_at = cursor_byte - byte_off;
                        let before = &t[..split_at];
                        let after = &t[split_at..];
                        if !before.is_empty() {
                            children.push(div().child(before.to_string()).into_any_element());
                        }
                        children.push(cursor_div());
                        if !after.is_empty() {
                            children.push(div().child(after.to_string()).into_any_element());
                        }
                    } else {
                        children.push(div().child(t.clone()).into_any_element());
                    }
                    byte_off = seg_end;
                }
                emoji_data::TextSegment::Emoji(u) => {
                    let unicode = emoji_data::unicode_from_u(u);
                    let seg_end = byte_off + unicode.len();
                    if show_cursor && cursor_byte == byte_off {
                        children.push(cursor_div());
                    }
                    if let Some(img_data) = self.emoji_image(u) {
                        children.push(
                            img(img_data)
                                .size(px(EMOJI_PX))
                                .flex_shrink_0()
                                .into_any_element(),
                        );
                    } else {
                        children.push(
                            div().child(unicode.clone()).into_any_element(),
                        );
                    }
                    byte_off = seg_end;
                }
            }
        }

        // Cursor at the end
        if show_cursor && cursor_byte >= byte_off {
            children.push(cursor_div());
        }

        // Placeholder when empty
        let is_empty = self.text.is_empty();

        let entity = cx.entity().clone();
        div()
            .id("emoji-input")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _window, cx| {
                let key = ev.keystroke.key.as_str();
                match key {
                    "backspace" => {
                        this.delete_backward();
                        cx.notify();
                    }
                    "delete" => {
                        this.delete_forward();
                        cx.notify();
                    }
                    "left" => {
                        this.move_cursor_left();
                        cx.notify();
                    }
                    "right" => {
                        this.move_cursor_right();
                        cx.notify();
                    }
                    "home" => {
                        this.cursor = 0;
                        this.blink_visible = true;
                        cx.notify();
                    }
                    "end" => {
                        this.cursor = this.text.len();
                        this.blink_visible = true;
                        cx.notify();
                    }
                    "enter" => {
                        let text = this.text.trim().to_string();
                        if !text.is_empty() {
                            cx.emit(EmojiInputSubmit(text));
                            this.clear();
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                this.focus.focus(window, cx);
                this.cursor = this.text.len();
                this.blink_visible = true;
                cx.notify();
            }))
            .flex()
            .flex_row()
            .items_center()
            .h(px(28.0))
            .px_2()
            .bg(rgb(0x18181b))
            .rounded(px(4.0))
            .border_1()
            .border_color(if focused { rgb(0x9b59b6) } else { rgb(0x2d2d30) })
            .text_xs()
            .text_color(rgb(0xefeff1))
            .overflow_hidden()
            .child(if is_empty && !focused {
                div()
                    .text_color(rgb(0x666666))
                    .child("Envoyer un message...")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .children(children)
                    .into_any_element()
            })
            .child({
                // Register as active input handler during paint so
                // IME/keyboard text is routed to EntityInputHandler.
                let entity_for_paint = entity.clone();
                let focus_for_paint = self.focus.clone();
                canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        window.handle_input(
                            &focus_for_paint,
                            ElementInputHandler::new(
                                Bounds::default(),
                                entity_for_paint.clone(),
                            ),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_0()
            })
    }
}

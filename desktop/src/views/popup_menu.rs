//! Interactive X11 popup menu overlay.
//!
//! A child window of the GPUI window, raised above mpv's child window so it
//! appears on top of the video. Draws a vertical list of text items with
//! hover highlighting and a checkmark on the selected row. Click emits an
//! OverlayEvent::MenuSelected.

#![cfg(target_os = "linux")]

use std::ffi::c_int;
use std::ffi::c_ulong;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use x11_dl::xlib::{self, Display, XButtonEvent, XEvent, XSetWindowAttributes, GC};

use crate::views::xft_text::{default_font, TextColor, XftRenderer};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MenuKind {
    Captions,
    Audio,
    Quality,
}

#[derive(Clone, Debug)]
pub enum MenuEvent {
    /// User clicked the item at `index` in the currently-open menu of `kind`.
    Selected { kind: MenuKind, index: usize },
}

const ROW_H: i32 = 32;
const PAD_X: i32 = 12;
const PAD_Y: i32 = 6;
const MIN_WIDTH: i32 = 200;
const MAX_VISIBLE_ROWS: usize = 10;

const BG: u64 = 0xff1c1c1f;
const HOVER_BG: u64 = 0xff26262b;
const TEXT: u64 = 0xffe8e8ea;
const ACCENT: u64 = 0xff9b59b6;
const BORDER: u64 = 0xff2d2d33;

pub struct PopupMenu {
    xlib: Arc<xlib::Xlib>,
    display: *mut Display,
    window: c_ulong,
    gc: GC,
    /// Antialiased text renderer (replaces the bitmap-font XDrawString).
    xft: Option<XftRenderer>,
    tx: Sender<MenuEvent>,
    visible: bool,
    kind: MenuKind,
    items: Vec<String>,
    selected_idx: Option<usize>,
    hover_idx: Option<usize>,
    /// Index of the first visible row (for scrolling).
    scroll_offset: usize,
    /// Number of rows actually rendered (≤ items.len(), ≤ MAX_VISIBLE_ROWS).
    visible_rows: usize,
    width: u32,
    height: u32,
}

unsafe impl Send for PopupMenu {}
unsafe impl Sync for PopupMenu {}

impl PopupMenu {
    pub fn new(parent_wid: c_ulong) -> Option<(Self, Receiver<MenuEvent>)> {
        let xlib = Arc::new(xlib::Xlib::open().ok()?);
        unsafe {
            let display = (xlib.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return None;
            }

            let mut attrs: XSetWindowAttributes = std::mem::zeroed();
            attrs.background_pixel = BG;
            attrs.border_pixel = BORDER;
            attrs.event_mask = xlib::ExposureMask
                | xlib::ButtonPressMask
                | xlib::PointerMotionMask
                | xlib::LeaveWindowMask;

            let window = (xlib.XCreateWindow)(
                display,
                parent_wid,
                0,
                0,
                MIN_WIDTH as u32,
                200,
                1,
                xlib::CopyFromParent,
                xlib::InputOutput as u32,
                std::ptr::null_mut(),
                xlib::CWBackPixel | xlib::CWBorderPixel | xlib::CWEventMask,
                &mut attrs,
            );

            let gc = (xlib.XCreateGC)(display, window, 0, std::ptr::null_mut());

            // Antialiased TrueType text via Xft
            let xft = XftRenderer::new(&xlib, display, window, &default_font(13));

            (xlib.XFlush)(display);

            let (tx, rx) = mpsc::channel::<MenuEvent>();

            Some((
                PopupMenu {
                    xlib,
                    display,
                    window,
                    gc,
                    xft,
                    tx,
                    visible: false,
                    kind: MenuKind::Quality,
                    items: Vec::new(),
                    selected_idx: None,
                    hover_idx: None,
                    scroll_offset: 0,
                    visible_rows: 0,
                    width: MIN_WIDTH as u32,
                    height: 200,
                },
                rx,
            ))
        }
    }

    /// Open the popup with the given items above the anchor (trigger button).
    /// `anchor_x` is the trigger's screen x (or parent-relative x), and the
    /// `anchor_bottom_y` is the y coordinate of the trigger's TOP edge — the
    /// popup places its bottom edge there so it opens upward.
    pub fn open(
        &mut self,
        kind: MenuKind,
        items: Vec<String>,
        selected_idx: Option<usize>,
        anchor_right_x: i32,
        anchor_top_y: i32,
    ) {
        self.kind = kind;
        self.items = items;
        self.selected_idx = selected_idx;
        self.hover_idx = None;
        // Scroll to keep the selected item visible (or to top if none).
        let total = self.items.len();
        self.visible_rows = total.min(MAX_VISIBLE_ROWS);
        self.scroll_offset = match selected_idx {
            Some(i) if i >= self.visible_rows => {
                (i + 1).saturating_sub(self.visible_rows)
            }
            _ => 0,
        };
        self.recompute_size();

        // Right-align: popup's right edge = anchor_right_x
        let x = (anchor_right_x - self.width as i32).max(4);
        // Upward: popup's bottom edge = anchor_top_y - 4 (small gap)
        let y = (anchor_top_y - 4 - self.height as i32).max(4);

        unsafe {
            (self.xlib.XMoveResizeWindow)(
                self.display,
                self.window,
                x,
                y,
                self.width,
                self.height,
            );
            if !self.visible {
                (self.xlib.XMapRaised)(self.display, self.window);
                self.visible = true;
            } else {
                (self.xlib.XRaiseWindow)(self.display, self.window);
            }
            (self.xlib.XFlush)(self.display);
        }
        self.redraw();
    }

    pub fn close(&mut self) {
        if self.visible {
            unsafe {
                (self.xlib.XUnmapWindow)(self.display, self.window);
                (self.xlib.XFlush)(self.display);
            }
            self.visible = false;
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn current_kind(&self) -> MenuKind {
        self.kind
    }

    fn recompute_size(&mut self) {
        let visible = self.visible_rows.max(1);
        self.height = (PAD_Y * 2 + ROW_H * visible as i32) as u32;

        let mut max_text = 0;
        for it in &self.items {
            let w = self.text_width(it);
            if w > max_text {
                max_text = w;
            }
        }
        self.width = ((max_text + PAD_X * 2 + 32).max(MIN_WIDTH)) as u32;
    }

    fn max_scroll(&self) -> usize {
        self.items.len().saturating_sub(self.visible_rows)
    }

    fn scroll_by(&mut self, delta: isize) {
        let new = (self.scroll_offset as isize + delta).max(0) as usize;
        let clamped = new.min(self.max_scroll());
        if clamped != self.scroll_offset {
            self.scroll_offset = clamped;
            self.redraw();
        }
    }

    fn text_width(&self, s: &str) -> i32 {
        match &self.xft {
            Some(x) => x.text_width(s),
            None => s.len() as i32 * 7,
        }
    }

    /// Drain pending X events for this window. Call periodically.
    pub fn pump(&mut self) {
        unsafe {
            while (self.xlib.XPending)(self.display) > 0 {
                let mut ev: XEvent = std::mem::zeroed();
                (self.xlib.XNextEvent)(self.display, &mut ev);
                match ev.get_type() {
                    xlib::Expose => self.redraw(),
                    xlib::ButtonPress => {
                        let be: XButtonEvent = ev.button;
                        if be.window == self.window {
                            match be.button {
                                xlib::Button1 => self.handle_click(be.y),
                                // Scroll wheel up / down (X11 buttons 4 / 5)
                                4 => self.scroll_by(-3),
                                5 => self.scroll_by(3),
                                _ => {}
                            }
                        }
                    }
                    xlib::MotionNotify => {
                        let me = ev.motion;
                        if me.window == self.window {
                            self.handle_motion(me.y);
                        }
                    }
                    xlib::LeaveNotify => {
                        if self.hover_idx.is_some() {
                            self.hover_idx = None;
                            self.redraw();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn row_at(&self, y: c_int) -> Option<usize> {
        let y = y - PAD_Y;
        if y < 0 {
            return None;
        }
        let visible_idx = (y / ROW_H) as usize;
        if visible_idx >= self.visible_rows {
            return None;
        }
        let idx = self.scroll_offset + visible_idx;
        if idx < self.items.len() {
            Some(idx)
        } else {
            None
        }
    }

    fn handle_motion(&mut self, y: c_int) {
        let new = self.row_at(y);
        if new != self.hover_idx {
            self.hover_idx = new;
            self.redraw();
        }
    }

    fn handle_click(&mut self, y: c_int) {
        if let Some(idx) = self.row_at(y) {
            let _ = self.tx.send(MenuEvent::Selected {
                kind: self.kind,
                index: idx,
            });
        }
    }

    fn redraw(&self) {
        unsafe {
            // Background
            (self.xlib.XSetForeground)(self.display, self.gc, BG);
            (self.xlib.XFillRectangle)(
                self.display,
                self.window,
                self.gc,
                0,
                0,
                self.width,
                self.height,
            );

            // Render only the visible window of items.
            let end = (self.scroll_offset + self.visible_rows).min(self.items.len());
            for (visible_idx, idx) in (self.scroll_offset..end).enumerate() {
                let item = &self.items[idx];
                let row_top = PAD_Y + (visible_idx as i32) * ROW_H;
                let is_hover = self.hover_idx == Some(idx);
                let is_sel = self.selected_idx == Some(idx);

                if is_hover {
                    (self.xlib.XSetForeground)(self.display, self.gc, HOVER_BG);
                    (self.xlib.XFillRectangle)(
                        self.display,
                        self.window,
                        self.gc,
                        4,
                        row_top,
                        self.width - 8,
                        ROW_H as u32,
                    );
                }

                if let Some(xft) = &self.xft {
                    // Vertically center text in row using font metrics.
                    let baseline = row_top + (ROW_H + xft.ascent() - xft.descent()) / 2;
                    let color = if is_sel { TextColor::Accent } else { TextColor::Primary };
                    xft.draw(PAD_X, baseline, item, color);
                    if is_sel {
                        xft.draw(
                            self.width as i32 - PAD_X - xft.text_width("✓"),
                            baseline,
                            "✓",
                            TextColor::Accent,
                        );
                    }
                }
                let _ = (TEXT, ACCENT); // silence unused warnings until removed
            }

            // Scrollbar (right edge) when the list overflows.
            if self.items.len() > self.visible_rows {
                let track_x = self.width as i32 - 4;
                let track_top = PAD_Y;
                let track_h = (self.height as i32) - PAD_Y * 2;
                (self.xlib.XSetForeground)(self.display, self.gc, 0xff2d2d33);
                (self.xlib.XFillRectangle)(
                    self.display,
                    self.window,
                    self.gc,
                    track_x,
                    track_top,
                    3,
                    track_h as u32,
                );
                let total = self.items.len() as i32;
                let thumb_h = ((track_h * self.visible_rows as i32) / total).max(20);
                let max_off = (total - self.visible_rows as i32).max(1);
                let thumb_y = track_top
                    + ((track_h - thumb_h) * self.scroll_offset as i32) / max_off;
                (self.xlib.XSetForeground)(self.display, self.gc, ACCENT);
                (self.xlib.XFillRectangle)(
                    self.display,
                    self.window,
                    self.gc,
                    track_x,
                    thumb_y,
                    3,
                    thumb_h as u32,
                );
            }

            (self.xlib.XFlush)(self.display);
        }
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        // Drop xft first so it can clean its draw/font/colors against the
        // still-valid display, then free the GC and window.
        self.xft = None;
        unsafe {
            (self.xlib.XFreeGC)(self.display, self.gc);
            (self.xlib.XDestroyWindow)(self.display, self.window);
            (self.xlib.XCloseDisplay)(self.display);
        }
    }
}

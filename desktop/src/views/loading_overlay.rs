//! Standalone X11 sibling window used as a "loading screen" overlay
//! placed above the mpv child window. It does NOT touch any mpv state —
//! it just covers mpv visually during a channel switch so the user
//! doesn't see the brief black-frame / freeze-frame artefact that mpv
//! produces when re-buffering a new file.
//!
//! mpv keeps decoding/playing exactly as before. Once the backup mpv
//! signals "first frame ready" (`MPV_EVENT_VIDEO_RECONFIG`), the caller
//! hides this overlay and the underlying mpv window becomes visible —
//! producing a clean visual transition.

use std::ffi::{c_ulong, CString};
use std::sync::Arc;
use x11_dl::xft::{Xft, XftColor, XftDraw, XftFont};
use x11_dl::xlib::{self, Display};
use x11_dl::xrender::XRenderColor;

pub struct LoadingOverlay {
    xlib: Arc<xlib::Xlib>,
    display: *mut Display,
    window: c_ulong,
    visible: bool,
    /// Last applied geometry (x, y, w, h). We only call XMoveResizeWindow
    /// when the area actually changes — resize spam is expensive on X11.
    last_area: Option<(i32, i32, u32, u32)>,
    // Xft for drawing the "Chargement…" label.
    xft: Option<Arc<Xft>>,
    draw: *mut XftDraw,
    font: *mut XftFont,
    color_text: Option<XftColor>,
}

unsafe impl Send for LoadingOverlay {}
unsafe impl Sync for LoadingOverlay {}

impl LoadingOverlay {
    pub fn new(
        parent_wid: c_ulong,
        xlib: Arc<xlib::Xlib>,
        display: *mut Display,
    ) -> Option<Self> {
        unsafe {
            let black = (xlib.XBlackPixel)(display, (xlib.XDefaultScreen)(display));
            let window = (xlib.XCreateSimpleWindow)(
                display,
                parent_wid,
                0,
                0,
                400,
                300,
                0,
                black,
                black,
            );
            // Don't map yet — only when the caller decides a switch is
            // taking too long for the previous frame to keep things
            // visually smooth.
            (xlib.XFlush)(display);

            // Set up Xft for the "Chargement…" text.
            let xft = Arc::new(Xft::open().ok()?);
            let screen = (xlib.XDefaultScreen)(display);
            let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
            (xlib.XGetWindowAttributes)(display, window, &mut attrs);
            let visual = attrs.visual;
            let colormap = attrs.colormap;

            // Use a generic sans pattern so fontconfig falls back to
            // whatever sans font is installed (Inter, Noto Sans, etc.).
            let pattern = CString::new("sans:size=14:weight=500").ok()?;
            let font = (xft.XftFontOpenName)(display, screen, pattern.as_ptr());
            if font.is_null() {
                eprintln!("[overlay] font load failed");
                return None;
            }
            let draw = (xft.XftDrawCreate)(display, window, visual, colormap);
            if draw.is_null() {
                (xft.XftFontClose)(display, font);
                return None;
            }

            // Light grey text against black background.
            let color_text = alloc_color(&xft, display, visual, colormap, 0xaa, 0xaa, 0xaa);

            Some(LoadingOverlay {
                xlib,
                display,
                window,
                visible: false,
                last_area: None,
                xft: Some(xft),
                draw,
                font,
                color_text,
            })
        }
    }

    pub fn set_geometry(&mut self, x: i32, y: i32, width: u32, height: u32) {
        let area = (x, y, width, height);
        if self.last_area == Some(area) {
            return;
        }
        self.last_area = Some(area);
        unsafe {
            (self.xlib.XMoveResizeWindow)(self.display, self.window, x, y, width, height);
            if self.visible {
                (self.xlib.XRaiseWindow)(self.display, self.window);
                self.redraw();
            }
            (self.xlib.XFlush)(self.display);
        }
    }

    /// Map + raise the overlay above mpv. Re-draws the label.
    pub fn show(&mut self) {
        if self.visible {
            return;
        }
        unsafe {
            (self.xlib.XMapRaised)(self.display, self.window);
            (self.xlib.XFlush)(self.display);
        }
        self.visible = true;
        // Tiny delay isn't needed: the next render frame will redraw,
        // but to be safe issue an immediate label paint.
        self.redraw();
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            (self.xlib.XUnmapWindow)(self.display, self.window);
            (self.xlib.XFlush)(self.display);
        }
        self.visible = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Draw "Chargement…" centered. Cheap to call repeatedly.
    fn redraw(&self) {
        let (Some(xft), Some(color)) = (self.xft.as_ref(), self.color_text.as_ref()) else {
            return;
        };
        let (_, _, w, h) = self.last_area.unwrap_or((0, 0, 400, 300));
        let text = "Chargement…";
        unsafe {
            // Center the text. Use ascent for vertical baseline.
            let ascent = (*self.font).ascent;
            let descent = (*self.font).descent;
            let mut extents: x11_dl::xrender::_XGlyphInfo = std::mem::zeroed();
            (xft.XftTextExtentsUtf8)(
                self.display,
                self.font,
                text.as_ptr(),
                text.len() as i32,
                &mut extents,
            );
            let text_w = extents.xOff as i32;
            let text_h = ascent + descent;
            let x = (w as i32 - text_w) / 2;
            let y = (h as i32 + text_h) / 2 - descent;
            (xft.XftDrawStringUtf8)(
                self.draw,
                color,
                self.font,
                x,
                y,
                text.as_ptr(),
                text.len() as i32,
            );
            (self.xlib.XFlush)(self.display);
        }
    }
}

impl Drop for LoadingOverlay {
    fn drop(&mut self) {
        unsafe {
            if let Some(xft) = self.xft.take() {
                if !self.draw.is_null() {
                    (xft.XftDrawDestroy)(self.draw);
                }
                if !self.font.is_null() {
                    (xft.XftFontClose)(self.display, self.font);
                }
            }
            (self.xlib.XDestroyWindow)(self.display, self.window);
            (self.xlib.XFlush)(self.display);
        }
    }
}

fn alloc_color(
    xft: &Xft,
    display: *mut Display,
    visual: *mut x11_dl::xlib::Visual,
    colormap: c_ulong,
    r: u16,
    g: u16,
    b: u16,
) -> Option<XftColor> {
    unsafe {
        let render_color = XRenderColor {
            red: r << 8,
            green: g << 8,
            blue: b << 8,
            alpha: 0xffff,
        };
        let mut color: XftColor = std::mem::zeroed();
        if (xft.XftColorAllocValue)(display, visual, colormap, &render_color, &mut color) == 0 {
            return None;
        }
        Some(color)
    }
}

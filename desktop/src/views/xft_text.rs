//! Xft (X FreeType) text rendering helper.
//!
//! Xlib's core `XDrawString` uses bitmap fonts from the 1980s with no
//! antialiasing — it's why our X11 popups looked bleedy. Xft uses FreeType
//! for proper antialiased TrueType / OpenType rendering with subpixel
//! positioning, exactly like every modern toolkit.
//!
//! One `XftRenderer` per X11 window (popup, tooltip).

#![cfg(target_os = "linux")]

use std::ffi::{c_ulong, CString};
use std::sync::Arc;
use x11_dl::xft::{Xft, XftColor, XftDraw, XftFont};
use x11_dl::xlib::{self, Display};
use x11_dl::xrender::{XRenderColor, _XGlyphInfo};

pub struct XftRenderer {
    xft: Arc<Xft>,
    display: *mut Display,
    draw: *mut XftDraw,
    font: *mut XftFont,
    color_text: XftColor,
    color_accent: XftColor,
    color_muted: XftColor,
}

unsafe impl Send for XftRenderer {}
unsafe impl Sync for XftRenderer {}

impl XftRenderer {
    /// Create an Xft renderer for the given X11 window.
    /// `font_pattern` is a fontconfig pattern, e.g. "Inter:size=12" or
    /// "system-ui:size=13:weight=500".
    pub fn new(
        xlib: &xlib::Xlib,
        display: *mut Display,
        window: c_ulong,
        font_pattern: &str,
    ) -> Option<Self> {
        let xft = Arc::new(Xft::open().ok()?);
        unsafe {
            let screen = (xlib.XDefaultScreen)(display);

            // Use the WINDOW's visual + colormap (not the screen default).
            // The popup may inherit an ARGB visual from its GPUI parent, in
            // which case Xft must use that visual to compute correct
            // pre-multiplied colors. Mismatch here makes text look dark.
            let mut attrs: x11_dl::xlib::XWindowAttributes = std::mem::zeroed();
            (xlib.XGetWindowAttributes)(display, window, &mut attrs);
            let visual = attrs.visual;
            let colormap = attrs.colormap;

            // Open font via fontconfig pattern
            let pattern_c = CString::new(font_pattern).ok()?;
            let font = (xft.XftFontOpenName)(display, screen, pattern_c.as_ptr());
            if font.is_null() {
                return None;
            }

            // Drawable wrapper
            let draw = (xft.XftDrawCreate)(display, window, visual, colormap);
            if draw.is_null() {
                (xft.XftFontClose)(display, font);
                return None;
            }

            // Allocate three colors (text/accent/muted) using the WINDOW's
            // visual (handles ARGB pre-multiplication automatically).
            let color_text = alloc_color(&xft, display, visual, colormap, 0xe8, 0xe8, 0xea)?;
            let color_accent = alloc_color(&xft, display, visual, colormap, 0x9b, 0x59, 0xb6)?;
            let color_muted = alloc_color(&xft, display, visual, colormap, 0x6b, 0x6b, 0x70)?;

            Some(XftRenderer {
                xft,
                display,
                draw,
                font,
                color_text,
                color_accent,
                color_muted,
            })
        }
    }

    /// Approximate text width in pixels (uses XftTextExtentsUtf8).
    pub fn text_width(&self, text: &str) -> i32 {
        unsafe {
            let mut extents: _XGlyphInfo = std::mem::zeroed();
            (self.xft.XftTextExtentsUtf8)(
                self.display,
                self.font,
                text.as_ptr(),
                text.len() as i32,
                &mut extents,
            );
            extents.xOff as i32
        }
    }

    /// Font ascent (used to compute baseline from top-y).
    pub fn ascent(&self) -> i32 {
        unsafe { (*self.font).ascent }
    }

    /// Font descent.
    pub fn descent(&self) -> i32 {
        unsafe { (*self.font).descent }
    }

    /// Total line height.
    #[allow(dead_code)]
    pub fn line_height(&self) -> i32 {
        self.ascent() + self.descent()
    }

    pub fn draw(&self, x: i32, y_baseline: i32, text: &str, color: TextColor) {
        let c = match color {
            TextColor::Primary => &self.color_text,
            TextColor::Accent => &self.color_accent,
            TextColor::Muted => &self.color_muted,
        };
        unsafe {
            (self.xft.XftDrawStringUtf8)(
                self.draw,
                c,
                self.font,
                x,
                y_baseline,
                text.as_ptr(),
                text.len() as i32,
            );
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)]
pub enum TextColor {
    Primary,
    Accent,
    Muted,
}

fn alloc_color(
    xft: &Xft,
    display: *mut Display,
    visual: *mut x11_dl::xlib::Visual,
    colormap: c_ulong,
    r: u8,
    g: u8,
    b: u8,
) -> Option<XftColor> {
    unsafe {
        let render = XRenderColor {
            red: (r as u16) << 8 | r as u16,
            green: (g as u16) << 8 | g as u16,
            blue: (b as u16) << 8 | b as u16,
            alpha: 0xffff,
        };
        let mut color: XftColor = std::mem::zeroed();
        let ok = (xft.XftColorAllocValue)(display, visual, colormap, &render, &mut color);
        if ok == 0 {
            None
        } else {
            Some(color)
        }
    }
}

impl Drop for XftRenderer {
    fn drop(&mut self) {
        unsafe {
            // Note: visual/colormap are owned by X server, not ours to free.
            // Colors and font + draw need explicit cleanup.
            let screen = 0; // doesn't matter for these calls
            let _ = screen;
            (self.xft.XftDrawDestroy)(self.draw);
            (self.xft.XftFontClose)(self.display, self.font);
        }
    }
}

/// Default fontconfig pattern matching the system's UI font.
pub fn default_font(size: u32) -> String {
    format!("system-ui,Inter,Cantarell,DejaVu Sans:size={}:antialias=true", size)
}

//! X11 override-redirect tooltip window.
//!
//! GPUI cannot draw above the mpv X11 child window (X11 composites children
//! on top of parent drawing). This creates a top-level X11 popup window that
//! sits above everything — exactly like a GTK/Qt tooltip would.

#[cfg(target_os = "linux")]
use std::ffi::c_ulong;
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use x11_dl::xlib::{self, Display, XSetWindowAttributes};

#[cfg(target_os = "linux")]
use crate::views::xft_text::{default_font, TextColor, XftRenderer};

#[cfg(target_os = "linux")]
pub struct TooltipOverlay {
    xlib: Arc<xlib::Xlib>,
    display: *mut Display,
    window: c_ulong,
    gc: xlib::GC,
    /// Antialiased text via Xft (replaces the bitmap XDrawString).
    xft: Option<XftRenderer>,
    visible: bool,
    current_text: String,
}

// SAFETY: used only from the GPUI main thread.
#[cfg(target_os = "linux")]
unsafe impl Send for TooltipOverlay {}
#[cfg(target_os = "linux")]
unsafe impl Sync for TooltipOverlay {}

#[cfg(target_os = "linux")]
impl TooltipOverlay {
    pub fn new() -> Option<Self> {
        let xlib = Arc::new(xlib::Xlib::open().ok()?);
        unsafe {
            let display = (xlib.XOpenDisplay)(std::ptr::null());
            if display.is_null() {
                return None;
            }
            let screen = (xlib.XDefaultScreen)(display);
            let root = (xlib.XRootWindow)(display, screen);

            // Create top-level window with override-redirect so the WM ignores it.
            // Alpha=0xff ensures opacity on ARGB visuals.
            let bg = 0xff2d2d30u64; // dark grey
            let mut attrs: XSetWindowAttributes = std::mem::zeroed();
            attrs.override_redirect = 1;
            attrs.background_pixel = bg;
            attrs.border_pixel = 0xff9b59b6; // purple

            let window = (xlib.XCreateWindow)(
                display,
                root,
                0,
                0,
                200,
                24,
                1, // border width
                xlib::CopyFromParent,
                xlib::InputOutput as u32,
                std::ptr::null_mut(),
                xlib::CWOverrideRedirect | xlib::CWBackPixel | xlib::CWBorderPixel,
                &mut attrs,
            );

            // Create a GC for drawing
            let gc = (xlib.XCreateGC)(display, window, 0, std::ptr::null_mut());

            // Antialiased text via Xft (TrueType + subpixel + FreeType hinting).
            let xft = XftRenderer::new(&xlib, display, window, &default_font(13));

            (xlib.XFlush)(display);

            Some(TooltipOverlay {
                xlib,
                display,
                window,
                gc,
                xft,
                visible: false,
                current_text: String::new(),
            })
        }
    }

    /// Show the tooltip at the given screen-coords with the given text.
    pub fn show(&mut self, text: &str, screen_x: i32, screen_y: i32) {
        if text == self.current_text && self.visible {
            // Just reposition
            unsafe {
                (self.xlib.XMoveWindow)(self.display, self.window, screen_x, screen_y);
                (self.xlib.XFlush)(self.display);
            }
            return;
        }
        self.current_text = text.to_string();

        let (text_w, ascent, descent) = match &self.xft {
            Some(x) => (x.text_width(text), x.ascent(), x.descent()),
            None => (text.len() as i32 * 7, 11, 3),
        };
        let padding_x = 10;
        let padding_y = 6;
        let win_w = (text_w + padding_x * 2).max(20) as u32;
        let win_h = (ascent + descent + padding_y * 2).max(18) as u32;

        unsafe {
            (self.xlib.XMoveResizeWindow)(
                self.display,
                self.window,
                screen_x,
                screen_y,
                win_w,
                win_h,
            );
            if !self.visible {
                (self.xlib.XMapRaised)(self.display, self.window);
                self.visible = true;
            } else {
                (self.xlib.XRaiseWindow)(self.display, self.window);
            }

            (self.xlib.XClearWindow)(self.display, self.window);
            if let Some(xft) = &self.xft {
                let baseline = padding_y + ascent;
                xft.draw(padding_x, baseline, text, TextColor::Primary);
            }
            (self.xlib.XFlush)(self.display);
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                (self.xlib.XUnmapWindow)(self.display, self.window);
                (self.xlib.XFlush)(self.display);
            }
            self.visible = false;
        }
    }

    /// Query the X server for the current pointer position in root-window coords.
    pub fn query_pointer(&self) -> Option<(i32, i32)> {
        unsafe {
            let screen = (self.xlib.XDefaultScreen)(self.display);
            let root = (self.xlib.XRootWindow)(self.display, screen);
            let mut root_ret: c_ulong = 0;
            let mut child_ret: c_ulong = 0;
            let mut root_x: i32 = 0;
            let mut root_y: i32 = 0;
            let mut win_x: i32 = 0;
            let mut win_y: i32 = 0;
            let mut mask: u32 = 0;
            let ok = (self.xlib.XQueryPointer)(
                self.display,
                root,
                &mut root_ret,
                &mut child_ret,
                &mut root_x,
                &mut root_y,
                &mut win_x,
                &mut win_y,
                &mut mask,
            );
            if ok != 0 {
                Some((root_x, root_y))
            } else {
                None
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for TooltipOverlay {
    fn drop(&mut self) {
        self.xft = None; // free Xft resources before display
        unsafe {
            (self.xlib.XFreeGC)(self.display, self.gc);
            (self.xlib.XDestroyWindow)(self.display, self.window);
            (self.xlib.XCloseDisplay)(self.display);
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub struct TooltipOverlay;

#[cfg(not(target_os = "linux"))]
impl TooltipOverlay {
    pub fn new() -> Option<Self> {
        Some(TooltipOverlay)
    }
    pub fn show(&mut self, _text: &str, _x: i32, _y: i32) {}
    pub fn hide(&mut self) {}
    pub fn query_pointer(&self) -> Option<(i32, i32)> {
        None
    }
}

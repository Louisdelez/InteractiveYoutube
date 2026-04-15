//! "Now playing" badge overlaid in the top-left of the video area.
//!
//! Implemented as a small X11 sibling window raised above mpv (mpv's
//! X11 child draws above any GPUI element, so we have to use a real
//! sibling X11 window to render anything over the video). Renders:
//!
//! - The channel's avatar as a round image (decoded from the in-memory
//!   JPG/PNG bytes already used by the sidebar).
//! - The channel name to the right of the avatar.
//!
//! Updated lazily — caller pushes a new (name, avatar bytes) when the
//! channel changes, and the next `redraw()` regenerates the cached
//! pixmap.

use image::GenericImageView;
use std::ffi::{c_ulong, CString};
use std::sync::Arc;
use tiny_skia::{Pixmap, Transform};
use x11_dl::xft::{Xft, XftColor, XftDraw, XftFont};
use x11_dl::xlib::{self, Display};
use x11_dl::xrender::XRenderColor;

const SVG_STAR_HOLLOW: &[u8] = include_bytes!("../../assets/icons/star.svg");
const SVG_STAR_FILLED: &[u8] = include_bytes!("../../assets/icons/star-filled.svg");

const BADGE_HEIGHT: u32 = 36;
const AVATAR_SIZE: u32 = 28;
const PADDING_X: u32 = 6;
const AVATAR_TEXT_GAP: u32 = 8;
const TEXT_STAR_GAP: u32 = 8;
const STAR_SIZE: u32 = 18;
const TEXT_RIGHT_PADDING: u32 = 8;

pub struct ChannelBadge {
    xlib: Arc<xlib::Xlib>,
    display: *mut Display,
    window: c_ulong,
    visible: bool,
    last_origin: Option<(i32, i32)>,
    /// (text, avatar bytes) — re-rendered whenever this changes.
    current: Option<(String, Vec<u8>)>,
    /// True if `current` changed since the last redraw, so we know to
    /// recompute the pixmap.
    dirty: bool,
    /// When the current channel was set — used to auto-hide the badge
    /// 4 s later (Apple TV / YouTube TV pattern).
    shown_at: Option<std::time::Instant>,
    /// Whether the current channel is in the user's favourites — drives
    /// the star icon's appearance (filled gold vs hollow grey).
    is_favorite: bool,
    /// Set to `true` by the X11 ButtonPress handler when the user
    /// clicked the star area. PlayerView's poll loop reads + clears
    /// this and emits an event to AppView.
    star_clicked: bool,
    /// Cached pixmap for the star (18×18), pre-rasterised in two
    /// styles (favourited / not). Bytes are stored separately so we
    /// can blit the right one on each redraw.
    star_filled: Vec<u8>,
    star_hollow: Vec<u8>,
    // Xft for the channel-name text.
    xft: Option<Arc<Xft>>,
    draw: *mut XftDraw,
    font: *mut XftFont,
    color_text: Option<XftColor>,
    // Cached decoded avatar (RGBA pre-multiplied, AVATAR_SIZE×AVATAR_SIZE).
    avatar_rgba: Option<Vec<u8>>,
    // Cached text width (recomputed when `current.text` changes).
    text_width: i32,
}

unsafe impl Send for ChannelBadge {}
unsafe impl Sync for ChannelBadge {}

impl ChannelBadge {
    pub fn new(parent_wid: c_ulong, xlib: Arc<xlib::Xlib>, display: *mut Display) -> Option<Self> {
        unsafe {
            // The GPUI parent window uses an ARGB visual on most
            // compositors, so XBlackPixel (== 0x00000000) means
            // FULLY TRANSPARENT, not opaque black — the user sees
            // straight through to the desktop. We need the alpha bits
            // set to 0xFF for opaque content.
            let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
            (xlib.XGetWindowAttributes)(display, parent_wid, &mut attrs);
            let parent_depth = attrs.depth;
            // Dark gray bg so the badge is visible against mpv's
            // black video area (a pure black bg would be invisible).
            // Top byte = alpha, must be 0xFF on ARGB visuals.
            let opaque_black: c_ulong = if parent_depth >= 32 {
                0xFF26262B
            } else {
                (xlib.XBlackPixel)(display, (xlib.XDefaultScreen)(display))
            };
            let window = (xlib.XCreateSimpleWindow)(
                display,
                parent_wid,
                10,
                10,
                200,
                BADGE_HEIGHT,
                0,
                opaque_black,
                opaque_black,
            );
            (xlib.XSetWindowBackground)(display, window, opaque_black);
            (xlib.XClearWindow)(display, window);
            // Subscribe to button-press events so we can detect clicks
            // on the star icon area.
            (xlib.XSelectInput)(display, window, xlib::ButtonPressMask);
            (xlib.XFlush)(display);

            let xft = Arc::new(Xft::open().ok()?);
            let screen = (xlib.XDefaultScreen)(display);
            let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
            (xlib.XGetWindowAttributes)(display, window, &mut attrs);
            let visual = attrs.visual;
            let colormap = attrs.colormap;

            let pattern = CString::new("sans:size=12:weight=600").ok()?;
            let font = (xft.XftFontOpenName)(display, screen, pattern.as_ptr());
            if font.is_null() {
                return None;
            }
            let draw = (xft.XftDrawCreate)(display, window, visual, colormap);
            if draw.is_null() {
                (xft.XftFontClose)(display, font);
                return None;
            }
            let color_text = alloc_color(&xft, display, visual, colormap, 0xff, 0xff, 0xff);

            Some(ChannelBadge {
                xlib,
                display,
                window,
                visible: false,
                last_origin: None,
                current: None,
                dirty: false,
                xft: Some(xft),
                draw,
                font,
                color_text,
                avatar_rgba: None,
                text_width: 0,
                shown_at: None,
                is_favorite: false,
                star_clicked: false,
                star_filled: rasterise_star(STAR_SIZE, true),
                star_hollow: rasterise_star(STAR_SIZE, false),
            })
        }
    }

    /// Update the displayed channel + favourite state. Triggers a
    /// redraw on the next `paint()` call.
    pub fn set_channel(&mut self, name: String, avatar_bytes: Vec<u8>, is_favorite: bool) {
        let same = self
            .current
            .as_ref()
            .map(|(n, b)| n == &name && b == &avatar_bytes)
            .unwrap_or(false);
        let fav_changed = self.is_favorite != is_favorite;
        self.is_favorite = is_favorite;
        if same && !fav_changed {
            // Same channel — refresh the visibility timer so the badge
            // re-shows for another 4 s if the avatar bytes arrive late.
            self.shown_at = Some(std::time::Instant::now());
            return;
        }
        if !same {
            self.current = Some((name, avatar_bytes));
            self.dirty = true;
        }
        self.shown_at = Some(std::time::Instant::now());
    }

    /// Drain X11 events on this window. Returns `true` if the user
    /// clicked on the star area since last call. Polled by PlayerView
    /// in its 60 Hz loop.
    pub fn poll_star_click(&mut self) -> bool {
        unsafe {
            let mut ev: xlib::XEvent = std::mem::zeroed();
            // ButtonPress events fire on this window — drain them all
            // and check x ranges against the star hit-box.
            while (self.xlib.XCheckWindowEvent)(
                self.display,
                self.window,
                xlib::ButtonPressMask,
                &mut ev,
            ) != 0
            {
                if ev.get_type() == xlib::ButtonPress {
                    let bp: &xlib::XButtonEvent = std::mem::transmute(&ev);
                    if bp.button == 1 {
                        // Hit-test the star region.
                        let star_x = (PADDING_X
                            + AVATAR_SIZE
                            + AVATAR_TEXT_GAP
                            + self.text_width.max(0) as u32
                            + TEXT_STAR_GAP) as i32;
                        let star_y = ((BADGE_HEIGHT - STAR_SIZE) / 2) as i32;
                        if bp.x >= star_x
                            && bp.x < star_x + STAR_SIZE as i32
                            && bp.y >= star_y
                            && bp.y < star_y + STAR_SIZE as i32
                        {
                            self.star_clicked = true;
                        }
                    }
                }
            }
        }
        let was = self.star_clicked;
        self.star_clicked = false;
        was
    }

    /// True iff the badge has been displayed for less than 4s. After
    /// that the caller should `hide()` it for a cleaner viewing
    /// experience.
    pub fn should_be_visible(&self) -> bool {
        self.shown_at
            .map(|t| t.elapsed() < std::time::Duration::from_secs(4))
            .unwrap_or(false)
    }

    /// Refresh the visibility timer so the badge stays up another 4s.
    /// Called when the user moves the mouse over the video area.
    pub fn bump(&mut self) {
        self.shown_at = Some(std::time::Instant::now());
    }

    /// Position the badge at (x, y) in the parent window. Resizes
    /// based on text width and repaints (X11 windows lose their
    /// content on every map / move / resize, so we re-paint each
    /// frame the player is rendered).
    pub fn place(&mut self, x: i32, y: i32) {
        if self.dirty {
            self.regenerate_cache();
        }
        let total_w = (PADDING_X
            + AVATAR_SIZE
            + AVATAR_TEXT_GAP
            + self.text_width.max(0) as u32
            + TEXT_STAR_GAP
            + STAR_SIZE
            + TEXT_RIGHT_PADDING)
            .max(80);
        let new_origin = (x, y);
        let resized = self.last_origin != Some(new_origin);
        unsafe {
            (self.xlib.XMoveResizeWindow)(
                self.display,
                self.window,
                x,
                y,
                total_w,
                BADGE_HEIGHT,
            );
            if self.visible {
                (self.xlib.XRaiseWindow)(self.display, self.window);
            }
            (self.xlib.XFlush)(self.display);
        }
        if resized {
            self.last_origin = Some(new_origin);
        }
        // Re-paint each frame — XMoveResizeWindow may invalidate the
        // window contents on some compositors and otherwise X11
        // doesn't restore them. Cheap (~ms).
        self.paint();
    }

    pub fn show(&mut self) {
        if !self.visible {
            unsafe {
                (self.xlib.XMapRaised)(self.display, self.window);
                (self.xlib.XFlush)(self.display);
            }
            self.visible = true;
        }
        self.paint();
    }

    pub fn is_visible(&self) -> bool {
        self.visible
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

    /// Draw the badge content (rounded avatar + name) into the X11
    /// window. Cheap to call repeatedly.
    pub fn paint(&self) {
        let Some(xft) = self.xft.as_ref() else { return };
        let Some(color) = self.color_text.as_ref() else { return };
        let Some((name, _)) = self.current.as_ref() else { return };

        // Background — must be opaque on ARGB visuals (alpha=0xFF).
        unsafe {
            let screen = (self.xlib.XDefaultScreen)(self.display);
            let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
            (self.xlib.XGetWindowAttributes)(self.display, self.window, &mut attrs);
            let depth = attrs.depth;
            let bg: c_ulong = if depth >= 32 {
                0xFF26262B
            } else {
                (self.xlib.XBlackPixel)(self.display, screen)
            };
            let gc = (self.xlib.XCreateGC)(
                self.display,
                self.window,
                0,
                std::ptr::null_mut(),
            );
            (self.xlib.XSetForeground)(self.display, gc, bg);
            (self.xlib.XFillRectangle)(
                self.display,
                self.window,
                gc,
                0,
                0,
                10000,
                BADGE_HEIGHT,
            );

            // Avatar — draw the cached ARGB buffer if we have one.
            if let Some(rgba) = self.avatar_rgba.as_ref() {
                draw_rgba_circle(
                    &self.xlib,
                    self.display,
                    self.window,
                    gc,
                    PADDING_X as i32,
                    ((BADGE_HEIGHT - AVATAR_SIZE) / 2) as i32,
                    AVATAR_SIZE,
                    rgba,
                    depth,
                );
            }
            // Channel name to the right of the avatar.
            let baseline_y = (BADGE_HEIGHT as i32 + (*self.font).ascent
                - (*self.font).descent)
                / 2;
            let text_x = (PADDING_X + AVATAR_SIZE + AVATAR_TEXT_GAP) as i32;
            (xft.XftDrawStringUtf8)(
                self.draw,
                color,
                self.font,
                text_x,
                baseline_y,
                name.as_ptr(),
                name.len() as i32,
            );

            // Star icon (favourite toggle).
            let star_x = (PADDING_X
                + AVATAR_SIZE
                + AVATAR_TEXT_GAP
                + self.text_width.max(0) as u32
                + TEXT_STAR_GAP) as i32;
            let star_y = ((BADGE_HEIGHT - STAR_SIZE) / 2) as i32;
            let rgba = if self.is_favorite {
                &self.star_filled
            } else {
                &self.star_hollow
            };
            draw_rgba_circle(
                &self.xlib,
                self.display,
                self.window,
                gc,
                star_x,
                star_y,
                STAR_SIZE,
                rgba,
                depth,
            );
            (self.xlib.XFreeGC)(self.display, gc);
            (self.xlib.XFlush)(self.display);
        }
    }

    fn regenerate_cache(&mut self) {
        self.dirty = false;
        let Some((name, avatar_bytes)) = self.current.clone() else { return };

        // Decode + resize avatar to AVATAR_SIZE.
        self.avatar_rgba = decode_and_resize(avatar_bytes.as_slice(), AVATAR_SIZE);

        // Measure text width.
        if let (Some(xft), font) = (self.xft.as_ref(), self.font) {
            unsafe {
                let mut extents: x11_dl::xrender::_XGlyphInfo = std::mem::zeroed();
                (xft.XftTextExtentsUtf8)(
                    self.display,
                    font,
                    name.as_ptr(),
                    name.len() as i32,
                    &mut extents,
                );
                self.text_width = extents.xOff as i32;
            }
        }
    }
}

impl Drop for ChannelBadge {
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

/// Decode JPG/PNG bytes, resize to `size×size`, mask to a circle, and
/// pre-multiply alpha so the output matches `tiny_skia`'s convention
/// (`draw_rgba_circle` expects premultiplied input).
fn decode_and_resize(bytes: &[u8], size: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let resized = img.resize_to_fill(size, size, image::imageops::FilterType::Triangle);
    let mut rgba = resized.to_rgba8().into_raw();
    let r = (size as f32) / 2.0;
    let cx = r;
    let cy = r;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let i = ((y * size + x) * 4) as usize;
            if d > r {
                // Outside the circle — fully transparent.
                rgba[i] = 0;
                rgba[i + 1] = 0;
                rgba[i + 2] = 0;
                rgba[i + 3] = 0;
            } else {
                // Image crate gives straight (un-premultiplied) alpha;
                // pre-multiply now so the same blend in
                // draw_rgba_circle works for both star (already
                // premultiplied via tiny_skia) and avatar.
                let a = rgba[i + 3] as u16;
                rgba[i] = ((rgba[i] as u16 * a) / 255) as u8;
                rgba[i + 1] = ((rgba[i + 1] as u16 * a) / 255) as u8;
                rgba[i + 2] = ((rgba[i + 2] as u16 * a) / 255) as u8;
            }
        }
    }
    let _ = img.dimensions();
    Some(rgba)
}

/// Blit an RGBA buffer into the window using XPutImage. Cheap (~few
/// hundred μs for 28×28).
unsafe fn draw_rgba_circle(
    xlib: &xlib::Xlib,
    display: *mut Display,
    window: c_ulong,
    gc: x11_dl::xlib::GC,
    x: i32,
    y: i32,
    size: u32,
    rgba: &[u8],
    depth: i32,
) {
    // Convert RGBA to ARGB BGRA expected by X11 24/32-bit visual.
    // XPutImage with depth=24 expects 4 bytes per pixel in ZPixmap
    // format (BGRX on little-endian). We pre-multiply by reading the
    // rgba and rearranging.
    // Blend the (premultiplied) RGBA input against the badge bg
    // colour so anti-aliased edges look smooth instead of "stair-
    // stepped" against the dark background.
    const BG_R: u8 = 0x26;
    const BG_G: u8 = 0x26;
    const BG_B: u8 = 0x2B;
    let len = (size * size) as usize;
    let mut bgra = vec![0u8; len * 4];
    for i in 0..len {
        let r = rgba[i * 4];
        let g = rgba[i * 4 + 1];
        let b = rgba[i * 4 + 2];
        let a = rgba[i * 4 + 3];
        // Output = src + (1 - alpha) * bg, where src is already
        // premultiplied (tiny_skia output convention).
        let inv = 255u16 - a as u16;
        let or = r as u16 + (BG_R as u16 * inv) / 255;
        let og = g as u16 + (BG_G as u16 * inv) / 255;
        let ob = b as u16 + (BG_B as u16 * inv) / 255;
        bgra[i * 4] = ob.min(255) as u8;
        bgra[i * 4 + 1] = og.min(255) as u8;
        bgra[i * 4 + 2] = or.min(255) as u8;
        bgra[i * 4 + 3] = 0xFF;
    }

    let visual = (xlib.XDefaultVisual)(display, (xlib.XDefaultScreen)(display));
    let img = (xlib.XCreateImage)(
        display,
        visual,
        depth as u32,
        x11_dl::xlib::ZPixmap,
        0,
        bgra.as_mut_ptr() as *mut _,
        size,
        size,
        32,
        0,
    );
    if img.is_null() {
        return;
    }
    (xlib.XPutImage)(display, window, gc, img, 0, 0, x, y, size, size);
    // XDestroyImage tries to free the data ptr too — we own bgra, so
    // null out the data ptr first to keep ownership.
    (*img).data = std::ptr::null_mut();
    (xlib.XDestroyImage)(img);
}

/// Rasterise a Lucide star SVG into an `size×size` RGBA buffer
/// suitable for `XPutImage`. `filled=true` uses the filled variant
/// (warm gold), `false` uses the hollow outline (white).
fn rasterise_star(size: u32, filled: bool) -> Vec<u8> {
    let svg = if filled { SVG_STAR_FILLED } else { SVG_STAR_HOLLOW };
    let opt = usvg::Options::default();
    let tree = match usvg::Tree::from_data(svg, &opt) {
        Ok(t) => t,
        Err(_) => return vec![0u8; (size * size * 4) as usize],
    };
    let mut pixmap = match Pixmap::new(size, size) {
        Some(p) => p,
        None => return vec![0u8; (size * size * 4) as usize],
    };
    let scale = size as f32 / tree.size().width();
    resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());

    // Recolour: lucide SVGs use `currentColor`, which the rasteriser
    // paints as opaque black. Replace every non-transparent pixel
    // with our chosen colour.
    let (cr, cg, cb) = if filled {
        (0xF5_u8, 0xC8_u8, 0x18_u8) // warm gold
    } else {
        (0xFF_u8, 0xFF_u8, 0xFF_u8) // white
    };
    let mut data = pixmap.data().to_vec();
    for px in data.chunks_exact_mut(4) {
        let a = px[3];
        if a == 0 {
            continue;
        }
        // Re-tint while preserving the alpha (anti-aliased edges stay
        // smooth). tiny_skia output is RGBA premultiplied.
        px[0] = ((cr as u16 * a as u16) / 255) as u8;
        px[1] = ((cg as u16 * a as u16) / 255) as u8;
        px[2] = ((cb as u16 * a as u16) / 255) as u8;
    }
    data
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

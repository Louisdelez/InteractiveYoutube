//! Dark-theme palette. Every semantic colour the UI uses has a named
//! const here — stop sprinkling `0x2d2d30` / `0x9b59b6` / `0xefeff1`
//! hex literals across the views (they were the top-3 most-used hex
//! codes in the repo before this module existed).
//!
//! Format matches GPUI's `rgb(u32)` helper: `0xRRGGBB`.

// ── Backgrounds ──────────────────────────────────────────────────────

/// Deepest chrome (app root + player area backdrop while loading).
pub const BACKGROUND: u32 = 0x0e0e10;
/// Topbar + bottom control bar backdrop.
pub const BAR_BG: u32 = 0x0f0f11;
/// Chat panel / sidebar / modal panel background.
pub const PANEL_BG: u32 = 0x18181b;
/// One notch lighter than PANEL — picker lists, emoji grid hover row.
pub const PANEL_RAISED: u32 = 0x17171a;
/// Button default background (and chip / pill background).
pub const BTN_BG: u32 = 0x26262b;
/// Button hover.
pub const BTN_HOVER: u32 = 0x33333a;
/// Button pressed / active.
pub const BTN_ACTIVE: u32 = 0x33333a;

// ── Borders ──────────────────────────────────────────────────────────

/// Main panel / divider border. The #1 most-used hex literal in the
/// old code (33 occurrences across views).
pub const BORDER: u32 = 0x2d2d30;
/// Tighter border for top/bottom chrome bars.
pub const BAR_BORDER: u32 = 0x1f1f23;

// ── Text ─────────────────────────────────────────────────────────────

/// Primary body text (high-contrast against BACKGROUND/PANEL).
pub const TEXT_PRIMARY: u32 = 0xefeff1;
/// Secondary text (hints, timestamps, meta).
pub const TEXT_MUTED: u32 = 0xaaaaaa;
/// Subtle text (watermarks, disabled states).
pub const TEXT_SUBTLE: u32 = 0x666666;
/// Near-black, rarely used directly — kept for CSS-like min-contrast.
pub const TEXT_STRONG: u32 = 0xe8e8ea;

// ── Accent (Twitch-violet brand) ─────────────────────────────────────

/// Primary brand colour.
pub const ACCENT: u32 = 0x9b59b6;
/// Hover variant of ACCENT.
pub const ACCENT_HOVER: u32 = 0xb57edc;
/// Light variant of ACCENT (eye / viewer counter icons).
pub const ACCENT_LIGHT: u32 = 0xbf94ff;

// ── Status colours ───────────────────────────────────────────────────

/// Error text / destructive action.
pub const ERROR: u32 = 0xef4444;
/// Warning text (maintenance banner).
pub const WARNING: u32 = 0xeab308;

// ── Primitives ───────────────────────────────────────────────────────

pub const WHITE: u32 = 0xffffff;
pub const BLACK: u32 = 0x000000;

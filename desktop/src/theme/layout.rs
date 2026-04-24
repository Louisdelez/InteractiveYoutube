//! App-level layout tokens. All in f32 pixels matching GPUI's `px()`.
//! These drive geometry of the X11 child windows (main mpv, backup,
//! overlays, badge) and the GPUI flex boxes around them. If they drift
//! you get "video covers the topbar" / "sidebar too narrow" bugs.

/// Left icon sidebar.
pub const SIDEBAR_W: f32 = 56.0;
/// Right chat panel width (when visible).
pub const CHAT_W: f32 = 340.0;
/// Top bar (title, connexion, programme, user menu).
pub const TOPBAR_H: f32 = 36.0;
/// Bottom control bar (play/pause, quality, captions, volume).
pub const CONTROL_BAR_H: f32 = 48.0;
/// Info bar below the video (title, published, YouTube link).
pub const INFOBAR_H: f32 = 36.0;

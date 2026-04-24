//! Single source of truth for layout + visual tokens shared across the
//! desktop views. Previously duplicated between `views/player.rs` and
//! `views/app.rs` with a `must match app.rs` comment (= drift-guaranteed).
//!
//! Import as:
//!     use crate::theme::{layout, colors};
//! or selectively:
//!     use crate::theme::colors::{ACCENT, BORDER, TEXT_PRIMARY};

pub mod colors;
pub mod layout;

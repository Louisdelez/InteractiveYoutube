//! Lucide SVG icons rasterized into GPUI Image assets at startup.
//!
//! Each icon is rasterized once at the requested pixel size. We pre-multiply
//! with a colour by recolouring non-transparent pixels — but because GPUI's
//! `img()` doesn't easily recolour an Image at render time, we generate one
//! Image per (icon, color) cache key.

use gpui::*;
use std::collections::HashMap;
use std::sync::Arc;
use tiny_skia::{Pixmap, Transform};

const SVG_PLAY: &[u8] = include_bytes!("../../assets/icons/play.svg");
const SVG_VOLUME_2: &[u8] = include_bytes!("../../assets/icons/volume-2.svg");
const SVG_VOLUME_X: &[u8] = include_bytes!("../../assets/icons/volume-x.svg");
const SVG_SUBS: &[u8] = include_bytes!("../../assets/icons/subtitles.svg");
const SVG_SUBS_OFF: &[u8] = include_bytes!("../../assets/icons/subtitles-off.svg");
const SVG_SETTINGS: &[u8] = include_bytes!("../../assets/icons/settings-2.svg");
const SVG_LANGUAGES: &[u8] = include_bytes!("../../assets/icons/languages.svg");
const SVG_YOUTUBE: &[u8] = include_bytes!("../../assets/icons/youtube.svg");
const SVG_SEARCH: &[u8] = include_bytes!("../../assets/icons/search.svg");
const SVG_HISTORY: &[u8] = include_bytes!("../../assets/icons/history.svg");
const SVG_TV: &[u8] = include_bytes!("../../assets/icons/tv.svg");
const SVG_STAR: &[u8] = include_bytes!("../../assets/icons/star.svg");
const SVG_GITHUB: &[u8] = include_bytes!("../../assets/icons/github.svg");
const SVG_EYE: &[u8] = include_bytes!("../../assets/icons/eye.svg");
const SVG_MSG: &[u8] = include_bytes!("../../assets/icons/message-square.svg");
const SVG_MSG_OFF: &[u8] = include_bytes!("../../assets/icons/message-square-off.svg");

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum IconName {
    Play,
    Volume,
    VolumeMute,
    Captions,
    CaptionsOff,
    Settings,
    Languages,
    Youtube,
    Search,
    History,
    Tv,
    Star,
    Github,
    Eye,
    MessageSquare,
    MessageSquareOff,
}

impl IconName {
    fn svg(self) -> &'static [u8] {
        match self {
            IconName::Play => SVG_PLAY,
            IconName::Volume => SVG_VOLUME_2,
            IconName::VolumeMute => SVG_VOLUME_X,
            IconName::Captions => SVG_SUBS,
            IconName::CaptionsOff => SVG_SUBS_OFF,
            IconName::Settings => SVG_SETTINGS,
            IconName::Languages => SVG_LANGUAGES,
            IconName::Youtube => SVG_YOUTUBE,
            IconName::Search => SVG_SEARCH,
            IconName::History => SVG_HISTORY,
            IconName::Tv => SVG_TV,
            IconName::Star => SVG_STAR,
            IconName::Github => SVG_GITHUB,
            IconName::Eye => SVG_EYE,
            IconName::MessageSquare => SVG_MSG,
            IconName::MessageSquareOff => SVG_MSG_OFF,
        }
    }
}

pub struct IconCache {
    /// (icon, size, color packed RGB) → image
    cache: HashMap<(IconName, u32, u32), Arc<Image>>,
}

impl IconCache {
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    /// Get a recoloured icon at the given pixel size.
    /// `color` is RGB packed (0xRRGGBB). Returns None if rasterization fails.
    pub fn get(&mut self, name: IconName, size: u32, color: u32) -> Option<Arc<Image>> {
        let key = (name, size, color);
        if let Some(img) = self.cache.get(&key) {
            return Some(img.clone());
        }

        let opt = usvg::Options::default();
        let tree = usvg::Tree::from_data(name.svg(), &opt).ok()?;
        let mut pixmap = Pixmap::new(size, size)?;
        let scale = size as f32 / tree.size().width();
        resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());

        // Recolour: Lucide SVGs use `currentColor` so the SVG renderer paints
        // them in default (often opaque black or transparent). After rasterization
        // we replace every non-transparent pixel with `color`, preserving alpha.
        let r = ((color >> 16) & 0xff) as u8;
        let g = ((color >> 8) & 0xff) as u8;
        let b = (color & 0xff) as u8;
        let mut data = pixmap.data().to_vec();
        for px in data.chunks_exact_mut(4) {
            // tiny-skia outputs RGBA premultiplied. Replace RGB while keeping alpha.
            let a = px[3];
            if a == 0 {
                continue;
            }
            // Premultiply the new RGB by alpha
            px[0] = ((r as u16 * a as u16) / 255) as u8;
            px[1] = ((g as u16 * a as u16) / 255) as u8;
            px[2] = ((b as u16 * a as u16) / 255) as u8;
        }

        let image = Arc::new(Image::from_bytes(ImageFormat::Png, encode_png(&data, size)?));
        self.cache.insert(key, image.clone());
        Some(image)
    }
}

/// Encode a raw RGBA buffer as PNG (gpui::Image takes encoded bytes).
fn encode_png(rgba: &[u8], size: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    drop(writer);
    Some(out)
}

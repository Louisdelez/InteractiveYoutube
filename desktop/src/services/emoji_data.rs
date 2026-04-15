//! Emoji dataset loaded from `assets/emojis-fr.json` (copied from
//! `emoji-picker-react`'s French data file). Provides the same categorized
//! emoji list used by the web app.

use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct Emoji {
    /// Unicode codepoints joined by `-`, e.g. `1f600` or `1f1eb-1f1f7`. This
    /// is also the filename (without `.png`) of the matching Apple PNG asset.
    pub u: String,
    /// First name (used as accessible label / tooltip).
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct EmojiCategory {
    /// Internal id (smileys_people, animals_nature, ...).
    #[allow(dead_code)]
    pub id: &'static str,
    /// Display name (in French) — for future use as tooltip on category tabs.
    #[allow(dead_code)]
    pub label: String,
    pub emojis: Vec<Emoji>,
}

const RAW: &str = include_str!("../../assets/emojis-fr.json");

/// Order of categories as they appear in the picker (matches web app order).
const CATEGORY_ORDER: &[&str] = &[
    "smileys_people",
    "animals_nature",
    "food_drink",
    "travel_places",
    "activities",
    "objects",
    "symbols",
    "flags",
];

static CACHE: OnceLock<Vec<EmojiCategory>> = OnceLock::new();

pub fn categories() -> &'static [EmojiCategory] {
    CACHE.get_or_init(load).as_slice()
}

fn load() -> Vec<EmojiCategory> {
    let v: serde_json::Value = serde_json::from_str(RAW).expect("emojis-fr.json invalid");
    let cats = v.get("categories").and_then(|c| c.as_object());
    let emojis = v.get("emojis").and_then(|e| e.as_object());
    let mut out = Vec::new();
    for id in CATEGORY_ORDER {
        let label = cats
            .and_then(|c| c.get(*id))
            .and_then(|o| o.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or(id)
            .to_string();
        let list = emojis
            .and_then(|e| e.get(*id))
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|item| {
                        let u = item.get("u")?.as_str()?.to_string();
                        let name = item
                            .get("n")
                            .and_then(|n| n.as_array())
                            .and_then(|n| n.first())
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some(Emoji { u, name })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        out.push(EmojiCategory {
            id,
            label,
            emojis: list,
        });
    }
    out
}

/// Convert a `u` string (e.g. `1f1eb-1f1f7`) into the actual emoji string.
/// Used so we can append the *unicode* emoji to the chat input (the picker
/// shows images, but the message must contain the real char).
pub fn unicode_from_u(u: &str) -> String {
    u.split('-')
        .filter_map(|hex| u32::from_str_radix(hex, 16).ok())
        .filter_map(char::from_u32)
        .collect()
}

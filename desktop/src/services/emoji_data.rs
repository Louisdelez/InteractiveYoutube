//! Emoji dataset loaded from `assets/emojis-fr.json` (copied from
//! `emoji-picker-react`'s French data file). Provides the same categorized
//! emoji list used by the web app, PLUS a reverse lookup table built from
//! every Apple-style PNG in `assets/emoji-png/` so chat messages can
//! render emoji characters as images instead of relying on the system font.

use std::collections::HashMap;
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
    let emojis_val = v.get("emojis").and_then(|e| e.as_object());

    // Collect all codepoints that are in the JSON so we can find extras.
    let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut out = Vec::new();
    for id in CATEGORY_ORDER {
        let label = cats
            .and_then(|c| c.get(*id))
            .and_then(|o| o.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or(id)
            .to_string();
        let list = emojis_val
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
                        known.insert(u.clone());
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

    // Scan ALL PNGs and add those missing from the JSON into extra
    // categories: "Teintes de peau" (skin tone variants) and
    // "Combinaisons" (ZWJ sequences, other extras).
    let emoji_dir = format!("{}/assets/emoji-png", env!("CARGO_MANIFEST_DIR"));
    let mut skin_tones: Vec<Emoji> = Vec::new();
    let mut combos: Vec<Emoji> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&emoji_dir) {
        let mut codes: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_suffix(".png")
                    .map(|s| s.to_string())
            })
            .collect();
        codes.sort();
        for code in codes {
            if known.contains(&code) {
                continue;
            }
            let is_skin = code.contains("1f3fb")
                || code.contains("1f3fc")
                || code.contains("1f3fd")
                || code.contains("1f3fe")
                || code.contains("1f3ff");
            let emoji = Emoji {
                u: code,
                name: String::new(),
            };
            if is_skin {
                skin_tones.push(emoji);
            } else {
                combos.push(emoji);
            }
        }
    }

    if !skin_tones.is_empty() {
        out.push(EmojiCategory {
            id: "skin_tones",
            label: "Teintes de peau".to_string(),
            emojis: skin_tones,
        });
    }
    if !combos.is_empty() {
        out.push(EmojiCategory {
            id: "combos",
            label: "Combinaisons".to_string(),
            emojis: combos,
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

// ── Reverse lookup: Unicode string → codepoint filename ──────────────

struct ReverseData {
    /// Emoji patterns sorted by byte length descending (longest first)
    /// so greedy matching finds combined sequences before individual glyphs.
    sorted: Vec<(String, String)>, // (unicode_string, codepoint_name)
}

static REVERSE: OnceLock<ReverseData> = OnceLock::new();

fn build_reverse() -> ReverseData {
    let emoji_dir = format!("{}/assets/emoji-png", env!("CARGO_MANIFEST_DIR"));
    let mut map: HashMap<String, String> = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&emoji_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if let Some(code) = s.strip_suffix(".png") {
                let unicode = unicode_from_u(code);
                if unicode.is_empty() {
                    continue;
                }
                map.insert(unicode.clone(), code.to_string());
                let stripped: String = unicode
                    .chars()
                    .filter(|c| *c != '\u{fe0f}')
                    .collect();
                if stripped != unicode && !stripped.is_empty() {
                    map.entry(stripped).or_insert_with(|| code.to_string());
                }
            }
        }
    }
    let mut sorted: Vec<(String, String)> = map.into_iter().collect();
    sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    ReverseData { sorted }
}

fn reverse_data() -> &'static ReverseData {
    REVERSE.get_or_init(build_reverse)
}

/// Segment types produced by `segment_text`.
pub enum TextSegment {
    Text(String),
    Emoji(String),
}

/// Split `text` into alternating plain-text and emoji runs.
/// Uses byte-level `str::find` (no char-index ambiguity) with
/// greedy longest-match: combined ZWJ/skin/flag sequences are
/// always tested before their individual glyphs.
pub fn segment_text(text: &str) -> Vec<TextSegment> {
    let rd = reverse_data();
    // Mark byte ranges that are emoji (non-overlapping, greedy).
    // emoji_at[byte_offset] = Some(codepoint_name) if this byte starts
    // an emoji, and the emoji extends `unicode.len()` bytes.
    let text_len = text.len();
    let mut covered = vec![false; text_len];
    // (start_byte, byte_len, codepoint)
    let mut hits: Vec<(usize, usize, String)> = Vec::new();
    for (unicode, code) in &rd.sorted {
        let pat = unicode.as_str();
        let pat_len = pat.len();
        let mut start = 0;
        while start < text_len {
            let Some(pos) = text[start..].find(pat) else { break };
            let abs = start + pos;
            if abs + pat_len > text_len { break; }
            if !covered[abs..abs + pat_len].iter().any(|&c| c) {
                for b in &mut covered[abs..abs + pat_len] {
                    *b = true;
                }
                hits.push((abs, pat_len, code.clone()));
            }
            start = abs + pat_len;
        }
    }
    hits.sort_by_key(|h| h.0);

    // Build segments
    let mut out: Vec<TextSegment> = Vec::new();
    let mut pos = 0;
    for (start, len, code) in &hits {
        if *start > pos {
            let between = &text[pos..*start];
            let clean: String = between
                .chars()
                .filter(|c| *c != '\u{fe0f}' && *c != '\u{200d}')
                .collect();
            if !clean.is_empty() {
                out.push(TextSegment::Text(clean));
            }
        }
        out.push(TextSegment::Emoji(code.clone()));
        pos = start + len;
    }
    if pos < text_len {
        let rest = &text[pos..];
        let clean: String = rest
            .chars()
            .filter(|c| *c != '\u{fe0f}' && *c != '\u{200d}')
            .collect();
        if !clean.is_empty() {
            out.push(TextSegment::Text(clean));
        }
    }
    out
}

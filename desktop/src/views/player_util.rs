//! Pure helper functions extracted from `views/player.rs`. Keeping
//! them in a small module lets `#[cfg(test)]` compile — appending
//! `#[test]` to the 2300-LOC `player.rs` tripped an `rustc` SIGSEGV
//! on GPUI macro expansion depth.
//!
//! These are the URL-parser / date-formatter helpers that don't
//! touch any GPUI / X11 / mpv state.

/// Extract the YouTube video ID from a URL like
/// `https://www.youtube.com/watch?v=XXXXXXXXXXX`. Returns `None` for
/// channel handles or non-watch URLs. YouTube video IDs are always
/// exactly 11 characters; anything else is rejected.
pub fn extract_video_id(url: &str) -> Option<String> {
    let after_v = url.split("watch?v=").nth(1)?;
    let id: String = after_v.chars().take_while(|c| *c != '&' && *c != '#').collect();
    if id.len() == 11 { Some(id) } else { None }
}

/// Open a URL in the user's default browser via `xdg-open`. Spawns a
/// detached thread that `wait()`s on the child so we don't leave
/// zombie processes in the table.
pub fn open_in_browser(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || {
        if let Ok(mut child) = std::process::Command::new("xdg-open").arg(&url).spawn() {
            let _ = child.wait();
        }
    });
}

/// Format a `publishedAt` ISO-8601 string (e.g. `"2019-02-12T15:00:00Z"`)
/// into a French tooltip like `"Mardi 12 février 2019 — il y a 6 ans"`.
/// Returns `None` for malformed input. Month-of-week is computed via
/// the Tomohiko Sakamoto variant (Zeller-like).
pub fn format_published_tooltip(iso: &str) -> Option<String> {
    let date_part = iso.get(..10)?;
    let parts: Vec<&str> = date_part.split('-').collect();
    if parts.len() != 3 { return None; }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) { return None; }

    let months_fr = [
        "", "janvier", "février", "mars", "avril", "mai", "juin",
        "juillet", "août", "septembre", "octobre", "novembre", "décembre",
    ];
    let days_fr = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];

    let t_tbl = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let dow = ((y + y / 4 - y / 100 + y / 400 + t_tbl[month as usize - 1] + day as i32)
        .rem_euclid(7)) as usize;
    let dow_mon = if dow == 0 { 6 } else { dow - 1 };
    let day_name = days_fr.get(dow_mon).copied().unwrap_or("");
    let month_name = months_fr.get(month as usize).copied().unwrap_or("");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let now_year = 1970 + (now.as_secs() / 31_557_600) as i32;
    let diff_years = now_year - year;
    let ago = if diff_years >= 2 {
        format!("il y a {} ans", diff_years)
    } else if diff_years == 1 {
        "il y a 1 an".to_string()
    } else {
        let now_month = ((now.as_secs() % 31_557_600) / 2_629_800) as u32 + 1;
        let diff_months = (now_year - year) as u32 * 12 + now_month.saturating_sub(month);
        if diff_months > 1 {
            format!("il y a {} mois", diff_months)
        } else if diff_months == 1 {
            "il y a 1 mois".to_string()
        } else {
            "récente".to_string()
        }
    };

    let day_name_cap = day_name
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
        + day_name.get(day_name.char_indices().nth(1).map(|(i, _)| i).unwrap_or(day_name.len())..).unwrap_or("");

    Some(format!(
        "{} {} {} {} — {}",
        day_name_cap, day, month_name, year, ago,
    ))
}

/// Map a 2-letter ISO-639-1 language code to a human-readable display
/// name (native). Falls through to the raw code if unknown.
pub fn lang_display_name(code: &str) -> &str {
    match code {
        "fr" => "Français",
        "en" => "English",
        "de" => "Deutsch",
        "es" => "Español",
        "it" => "Italiano",
        "pt" => "Português",
        "ru" => "Русский",
        "ja" => "日本語",
        "ko" => "한국어",
        "zh" => "中文",
        "ar" => "العربية",
        "nl" => "Nederlands",
        "pl" => "Polski",
        "tr" => "Türkçe",
        "sv" => "Svenska",
        "no" => "Norsk",
        "da" => "Dansk",
        "fi" => "Suomi",
        "el" => "Ελληνικά",
        "he" => "עברית",
        "hi" => "हिन्दी",
        "th" => "ไทย",
        "vi" => "Tiếng Việt",
        "uk" => "Українська",
        "cs" => "Čeština",
        "hu" => "Magyar",
        "ro" => "Română",
        "id" => "Indonesia",
        _ => code,
    }
}

/// Emit a dual-quality fallback debug line through the shared tracing
/// logger. Routed under target `quality` so log viewers can filter.
/// Replaces the old `/tmp/iyt-quality.log` hardcoded-path logger.
pub fn log_quality(msg: &str) {
    tracing::debug!(target: "quality", "{}", msg);
}

/// Locate bundled mpv user-shaders next to the executable. Returns a
/// colon-separated path string for mpv's `glsl-shaders` or None if no
/// shaders are bundled (dev layout falls through to the project
/// `assets/shaders/` dir).
pub fn bundled_shader_paths() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidates = [
        exe_dir.join("shaders"),
        exe_dir.join("../../assets/shaders"),
        exe_dir.join("../../../assets/shaders"),
    ];
    for dir in candidates {
        if !dir.is_dir() { continue; }
        let mut paths = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("glsl") {
                    if let Some(s) = p.to_str() { paths.push(s.to_string()); }
                }
            }
        }
        if !paths.is_empty() {
            paths.sort();
            return Some(paths.join(":"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_video_id_standard() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }
    #[test]
    fn extract_video_id_with_query_params() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }
    #[test]
    fn extract_video_id_with_fragment() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ#t=42"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }
    #[test]
    fn extract_video_id_rejects_short() {
        assert_eq!(extract_video_id("https://www.youtube.com/watch?v=short"), None);
    }
    #[test]
    fn extract_video_id_rejects_channel_handle() {
        assert_eq!(extract_video_id("https://www.youtube.com/@amixem"), None);
    }
    #[test]
    fn extract_video_id_rejects_non_watch_url() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/channel/UCgvqvBoSHB1ctlyyhoHrGwQ"),
            None
        );
    }

    #[test]
    fn format_published_tooltip_parses_valid_iso() {
        let out = format_published_tooltip("2019-02-12T15:00:00Z").expect("should parse");
        assert!(out.contains("février"));
        assert!(out.contains("2019"));
        assert!(out.contains("il y a") || out.contains("récente"));
    }
    #[test]
    fn format_published_tooltip_starts_with_capital() {
        let out = format_published_tooltip("2019-02-12T00:00:00Z").expect("should parse");
        let first = out.chars().next().unwrap();
        assert!(first.is_uppercase(), "expected leading uppercase: {:?}", out);
    }
    #[test]
    fn format_published_tooltip_rejects_garbage() {
        assert_eq!(format_published_tooltip(""), None);
        assert_eq!(format_published_tooltip("not-a-date"), None);
        assert_eq!(format_published_tooltip("2019-13-99T00:00:00Z"), None);
    }

    #[test]
    fn lang_display_name_known() {
        assert_eq!(lang_display_name("fr"), "Français");
        assert_eq!(lang_display_name("en"), "English");
        assert_eq!(lang_display_name("ja"), "日本語");
    }
    #[test]
    fn lang_display_name_unknown_passthrough() {
        assert_eq!(lang_display_name("xx"), "xx");
        assert_eq!(lang_display_name(""), "");
    }
}

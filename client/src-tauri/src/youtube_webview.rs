use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use wry::{Rect, WebView, WebViewBuilder};

// Wrap WebView in an unsafe Sync wrapper since we only access it from the main thread
pub struct WebViewWrapper(pub Option<WebView>);
unsafe impl Send for WebViewWrapper {}
unsafe impl Sync for WebViewWrapper {}

pub struct YoutubeState(pub Mutex<WebViewWrapper>);

const YOUTUBE_INJECT_JS: &str = r#"
(function() {
  if (!window.location.hostname.includes('youtube.com')) return;

  document.cookie = 'SOCS=CAI; domain=.youtube.com; path=/; max-age=31536000; SameSite=Lax';
  document.cookie = 'CONSENT=YES+cb.20210328-17-p0.en+FX+987; domain=.youtube.com; path=/; max-age=31536000; SameSite=Lax';

  function tryAcceptConsent() {
    const btns = document.querySelectorAll('button');
    for (const btn of btns) {
      const t = (btn.textContent || '').toLowerCase();
      if (t.includes('accept') || t.includes('accepter') || t.includes('tout accepter')) {
        btn.click(); return true;
      }
    }
    const form = document.querySelector('form[action*="consent"]');
    if (form) { form.submit(); return true; }
    return false;
  }
  let tries = 0;
  const ci = setInterval(() => { if (tryAcceptConsent() || tries++ > 30) clearInterval(ci); }, 200);

  const style = document.createElement('style');
  style.textContent = `
    ytd-masthead, #masthead-container,
    #comments, #related, #secondary,
    ytd-merch-shelf-renderer, #purchase-button,
    ytd-engagement-panel-section-list-renderer,
    #below, #watch-action-panels, ytd-watch-metadata, #meta,
    tp-yt-app-drawer, ytd-mini-guide-renderer, #guide-button,
    .ytp-ce-element, .ytp-endscreen-content, .ytp-chrome-top,
    ytd-watch-next-secondary-results-renderer,
    #secondary-inner { display: none !important; }
    html, body { overflow: hidden !important; margin: 0 !important; padding: 0 !important; background: #000 !important; }
    ytd-app { overflow: hidden !important; }
    #player-container-outer, #player-container-inner,
    #player-container, #movie_player, .html5-video-container,
    ytd-player, #player-theater-container, #full-bleed-container {
      position: fixed !important; top: 0 !important; left: 0 !important;
      width: 100vw !important; height: 100vh !important;
      max-width: 100vw !important; max-height: 100vh !important;
      min-height: 100vh !important;
    }
    video { object-fit: contain !important; width: 100% !important; height: 100% !important; }
    ytd-watch-flexy { --ytd-watch-flexy-max-player-width: 100vw !important; }
  `;
  document.head.appendChild(style);

  const vi = setInterval(() => {
    const video = document.querySelector('video');
    if (!video) return;
    clearInterval(vi);
    video.play().catch(() => {});
    setTimeout(() => { const tb = document.querySelector('.ytp-size-button'); if (tb) tb.click(); }, 1500);
    window.__IYT__ = {
      seek: (t) => { video.currentTime = t; },
      play: () => video.play().catch(() => {}),
      mute: () => { video.muted = true; },
      unmute: () => { video.muted = false; },
      setVolume: (v) => { video.volume = v / 100; },
    };
  }, 300);
})();
"#;

#[tauri::command]
pub fn create_youtube_webview(
    app: AppHandle,
    state: State<'_, YoutubeState>,
    video_id: String,
    x: f64, y: f64, width: f64, height: f64,
) -> Result<(), String> {
    let url = format!("https://www.youtube.com/watch?v={}&autoplay=1", video_id);

    let mut guard = state.0.lock().map_err(|e| e.to_string())?;

    // If exists, navigate
    if let Some(ref wv) = guard.0 {
        wv.load_url(&url).map_err(|e| e.to_string())?;
        return Ok(());
    }

    let main_window = app.get_webview_window("main").ok_or("No main window")?;

    let webview = WebViewBuilder::new()
        .with_url(&url)
        .with_bounds(Rect {
            position: wry::dpi::Position::Logical(wry::dpi::LogicalPosition::new(x, y)),
            size: wry::dpi::Size::Logical(wry::dpi::LogicalSize::new(width, height)),
        })
        .with_initialization_script(YOUTUBE_INJECT_JS)
        .with_autoplay(true)
        .build_as_child(&main_window)
        .map_err(|e| e.to_string())?;

    guard.0 = Some(webview);
    Ok(())
}

#[tauri::command]
pub fn youtube_navigate(state: State<'_, YoutubeState>, video_id: String) -> Result<(), String> {
    let url = format!("https://www.youtube.com/watch?v={}&autoplay=1", video_id);
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 {
        wv.load_url(&url).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn youtube_seek(state: State<'_, YoutubeState>, seconds: f64) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 {
        wv.evaluate_script(&format!("window.__IYT__?.seek({})", seconds)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn youtube_set_volume(state: State<'_, YoutubeState>, volume: u8) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 {
        wv.evaluate_script(&format!("window.__IYT__?.setVolume({})", volume)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn youtube_mute(state: State<'_, YoutubeState>) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 { wv.evaluate_script("window.__IYT__?.mute()").map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
pub fn youtube_unmute(state: State<'_, YoutubeState>) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 { wv.evaluate_script("window.__IYT__?.unmute()").map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
pub fn youtube_resize(state: State<'_, YoutubeState>, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 {
        wv.set_bounds(Rect {
            position: wry::dpi::Position::Logical(wry::dpi::LogicalPosition::new(x, y)),
            size: wry::dpi::Size::Logical(wry::dpi::LogicalSize::new(width, height)),
        }).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn youtube_destroy(state: State<'_, YoutubeState>) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    guard.0 = None;
    Ok(())
}

#[tauri::command]
pub fn youtube_show(state: State<'_, YoutubeState>) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 { wv.set_visible(true).map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
pub fn youtube_hide(state: State<'_, YoutubeState>) -> Result<(), String> {
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = guard.0 { wv.set_visible(false).map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
pub fn get_video_backends() -> Vec<String> {
    vec!["youtube".to_string()]
}

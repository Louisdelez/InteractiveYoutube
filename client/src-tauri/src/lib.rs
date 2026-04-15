mod youtube_webview;

use std::sync::Mutex;
use youtube_webview::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(YoutubeState(Mutex::new(youtube_webview::WebViewWrapper(None))))
        .invoke_handler(tauri::generate_handler![
            create_youtube_webview,
            youtube_navigate,
            youtube_seek,
            youtube_set_volume,
            youtube_mute,
            youtube_unmute,
            youtube_resize,
            youtube_destroy,
            youtube_show,
            youtube_hide,
            get_video_backends,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

mod app;
mod config;
mod i18n;
mod models;
mod services;
mod theme;
mod views;

use gpui::*;
use gpui_component::Root;

fn main() {
    // Initialise structured logging first so every subsequent step is
    // captured (stdout + rotated file in $XDG_DATA_HOME/KoalaTV/logs/
    // + best-effort forward to server /api/logs). Panics are hooked too.
    services::logger::init();

    // Ensure deno (required by yt-dlp for YouTube JS extraction) is in
    // PATH. yt-dlp looks for it via $PATH; without it, video URLs fail
    // to resolve silently inside mpv's ytdl_hook.
    if let Some(home) = std::env::var_os("HOME") {
        let deno_bin = std::path::PathBuf::from(&home).join(".deno/bin");
        if deno_bin.exists() {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = std::env::split_paths(&path).collect::<Vec<_>>();
            if !paths.contains(&deno_bin) {
                paths.insert(0, deno_bin);
                std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
            }
        }
    }

    // Keep yt-dlp fresh in the background — download if missing, self-update
    // on launch, then every 6 h. The resolved path is passed to mpv below.
    services::ytdlp_updater::spawn();

    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);
        // Force dark theme so Input/Slider/Popover use dark backgrounds and
        // light text instead of the default light theme.
        gpui_component::theme::Theme::change(
            gpui_component::theme::ThemeMode::Dark,
            None,
            cx,
        );

        cx.spawn(async move |cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(100.0), px(100.0)),
                        size: size(px(1280.0), px(800.0)),
                    })),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Koala TV".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = app::AppView::new(window, cx);
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Failed to open window");
        })
        .detach();
    });
}

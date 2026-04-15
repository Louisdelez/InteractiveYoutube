mod app;
mod config;
mod models;
mod views;
mod services;

use gpui::*;
use gpui_component::Root;

fn main() {
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

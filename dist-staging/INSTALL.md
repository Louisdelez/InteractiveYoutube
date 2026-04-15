# Koala TV — Windows x86_64 install

## Run

1. Extract this zip anywhere (keep `koala-tv.exe` and `libmpv-2.dll` together in the same folder).
2. Double-click `koala-tv.exe`.
3. First launch downloads `yt-dlp` into `%APPDATA%\KoalaTV\bin\` — this is normal, give it a few seconds.

## Known issue — v1.0.0-windows-beta.0

**mpv opens in a separate window.** The player area in the main Koala TV window is empty; the video shows in a second window next to it. This is a known limitation of this beta: the in-window mpv embed (via `wid=HWND`) is not yet implemented (Phase 2 of the cross-platform port, see `docs/CROSS_PLATFORM.md` in the repo).

What still works:
- Sidebar, chat, topbar, auth, settings — all functional.
- Playback itself — mpv plays the stream correctly, just in its own window.

What doesn't work yet on Windows:
- In-window video embedding.
- X11-specific overlays (tooltip, popup menu, channel badge, loading overlay) — all Linux-only for now.

## Feedback

Report issues at https://github.com/Louisdelez/KoalaTV/issues — mention `windows-beta.0` so we can triage.

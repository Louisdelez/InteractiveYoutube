# Desktop app reference

Rust (edition 2021) + [GPUI](https://www.gpui.rs) (Zed's UI framework) + [libmpv2](https://crates.io/crates/libmpv2). Linux-first (X11); the codebase compiles on Windows and macOS via `cfg(target_os = "linux")` gates but the in-window mpv embed and native overlays (popup menus, tooltip, badge, backup player) are Linux-only for now. See [CROSS_PLATFORM.md](CROSS_PLATFORM.md) for the per-platform status and roadmap.

## Why a native app?

- Browser YouTube iframes forbid some videos (copyright / creator setting). libmpv + yt-dlp plays any public video, so the TV never "skips" on a non-embeddable.
- Lower overhead than Chromium: a few hundred MB of RSS for 1080p playback vs. Chromium's gigabyte.
- Full control over subtitle tracks, audio tracks, quality selection.

## Entry

`src/main.rs`:

1. Spawns the yt-dlp auto-updater daemon thread (downloads / `-U` / sleep 6 h / repeat).
2. `gpui_platform::application().run(...)` — X11/Wayland backend init.
3. Forces `ThemeMode::Dark` so all gpui-component widgets use a dark palette.
4. Opens a 1280×800 window titled "Koala TV" and mounts `AppView` wrapped in `gpui_component::Root`.

## AppView (`src/app.rs`)

```
┌─ Top bar (36 px) ────────────────────────────────────────┐
│  title · search · gear · signal_bars · fps · auth        │
├─ Main (flex 1) ──────────────────────────────────────────┤
│  sidebar 56 px · player (mpv embed) · chat 340 px        │
└──────────────────────────────────────────────────────────┘
```

State:
- `sidebar`, `player`, `chat`: child entities.
- `latency_ms: Option<u32>` from a /health ping every 2 s. Drives the signal bars; hover shows `x ms` in the shared X11 tooltip.
- `frame_times: VecDeque<Instant>`: rolling 1-second window of render() calls. `len()` = current FPS. A 200 ms self-notify keeps the counter updating even when the rest of the UI is idle.
- `hovered_channel` + `tooltip: Rc<RefCell<Option<TooltipOverlay>>>`: the shared X11 override-redirect tooltip window (above mpv).
- `auth`, `settings_modal`: overlays that replace the chat panel when open.
- `settings: Settings`: loaded from `~/.config/koala-tv/settings.json`, synced to the server when logged in.

Subscriptions (GPUI's event system):
- `ChannelSelected(id, name, handle)` → emits `ClientCommand::SwitchChannel(id)` to the websocket thread, clears chat, updates sidebar.
- `ChannelHovered(Option<name>)` → shows/hides the tooltip (debounced 80 ms).
- `ChannelFavoriteToggle(id)` → toggles in `settings.favorites`, saves, pushes to server.
- `ChatSend(text)` → `ClientCommand::SendChat(text)`.
- `MemoryChanged(ids)` → sidebar's "Mémoire" section.
- `AutoAdvanced` → `ClientCommand::RequestState` (resync after the player auto-plays the next video).

Server events arrive on an `mpsc::Receiver<ServerEvent>` polled in an async loop:
- `TvState(state)` / `TvSync(state)` → `player.load_state(&state, cx)`.
- `ChatMessage{user, text, color}` → `chat.push_message(...)`.
- `ChatHistory(msgs)` → `chat.replace_messages(msgs)`.
- `ChatCleared` → `chat.replace_messages(vec![])`.
- `ViewerCount{count}` → `chat.set_viewer_count(count)`.
- `Connected` / `Disconnected` → sets a flag that shows a "server unavailable" curtain on the player.

## The mpv embed (`src/views/player.rs`)

The most delicate piece. A libmpv2 `Mpv` instance renders directly into an X11 child window (`wid=<child_window>`). mpv fetches the stream via yt-dlp (our auto-updated binary).

### Initialisation highlights

```rust
init.set_property("wid", child_window as i64)?;
init.set_property("ytdl", "yes")?;
init.set_property("ytdl-format", QUALITIES[0].1)?; // Auto

// Gap-free EOF → next via loadfile append-play
init.set_property("prefetch-playlist", "yes")?;
init.set_property("cache", "yes")?;
init.set_property("cache-secs", 60i64)?;
init.set_property("demuxer-readahead-secs", 20i64)?;
init.set_property("demuxer-max-bytes", "200MiB")?;
init.set_property("stream-lavf-o",
    "reconnect=1,reconnect_streamed=1,reconnect_delay_max=5")?;

// Keyframe-only seek = no decoder flush, no black flash
init.set_property("hr-seek", "no")?;
init.set_property("network-timeout", 10i64)?;

// GPU renderer, browser-like (no sharpening, no tone mapping)
init.set_property("vo", "gpu-next")?;
init.set_property("profile", "fast")?;
init.set_property("scale", "bilinear")?;

// Video-sync/interpolation tuned for VOD (no "soap opera")
init.set_property("video-sync", "audio")?;
init.set_property("interpolation", "no")?;
init.set_property("video-latency-hacks", "yes")?;

// Subtitles: fetch all languages, keep off by default
init.set_property("sub-visibility", false)?;
init.set_property("sub-auto", "all")?;
init.set_property("ytdl-raw-options",
    "sub-langs=all,write-auto-subs=,write-subs=")?;

// Use our auto-updated yt-dlp instead of $PATH
let ytdl_path = crate::services::ytdlp_updater::binary_path();
if ytdl_path.exists() {
    init.set_property("script-opts",
        format!("ytdl_hook-ytdl_path={}", ytdl_path.display()).as_str())?;
}
```

### Dual-mpv "zero cut" pattern

Two `Mpv` instances run side by side:
- **Main**: the quality the user selected (Auto / 1080p / 720p …).
- **Backup** (`src/views/backup_player.rs`): `worst[height<=360]/worst`, muted, in a separate X11 child window, kept offscreen until needed.

On every channel switch:

1. The server's `TvState` arrives.
2. The **backup** loads the new video at the right seek and starts decoding, still hidden.
3. The **main** `loadfile` is **deferred** until the backup has its first decoded frame (`EVENT::VideoReconfig` — the canonical "first frame" signal, not `PlaybackRestart`).
4. A loading overlay (black X11 sibling window with "Chargement…") appears if the backup takes > 400 ms; it stays visible for at least 500 ms to avoid sub-frame flicker.
5. Once the backup is on screen, the main `loadfile` kicks in.
6. When the main produces `VIDEO_RECONFIG`, we **swap up**: audio crossfades 120 ms, we move the backup offscreen (not `XUnmap`, to avoid an expose-black flash), and drift > 300 ms triggers a `seek` on the main to match.
7. If the main stalls mid-playback (`paused-for-cache=true` for > 2 s), we **swap down** (show the backup, crossfade audio 80 ms, seek if drift > 500 ms).

This is why you never see a black frame between channels — even on slow networks.

### Channel memory

A small LRU of "frozen" backup mpvs (one per visited channel) is kept in `memory_cache.rs`. Capacity is user-configurable (0 = disabled, default 2). Zapping back to a recent channel reveals its frozen backup instantly while the main reloads.

### Quality / audio / subtitle menus

Not gpui-component popovers — they're native X11 windows (`src/views/popup_menu.rs`). They're raised above mpv with their own event loop (`XButtonPress`, `PointerMotionMask`, `ExposureMask`). Hover / click / scroll handled natively. Antialiased text via Xft. They emit `MenuEvent::Selected { kind, index }` back to the Rust side.

This is done natively because GPUI's popovers would render under the mpv X11 window (mpv has its own compositing path).

### Subtitle language filter

YouTube's auto-translated subtitles expose ~200 languages. We pass `sub-langs=all` to yt-dlp, but in the UI we show only 5 common languages (fr, en, de, es, it) by default with a "Plus de langues" option to expand.

## Native X11 bits (all `cfg(target_os = "linux")`)

| File                          | Role                                                                 |
| ----------------------------- | -------------------------------------------------------------------- |
| `views/tooltip.rs`            | Override-redirect tooltip window with antialiased text (Xft)         |
| `views/popup_menu.rs`         | Native popup menus (quality / audio / subtitles)                     |
| `views/channel_badge.rs`      | Top-left overlay: avatar · name · ⭐ (if favorite)                   |
| `views/loading_overlay.rs`    | Black overlay with "Chargement…" during channel switch              |
| `views/backup_player.rs`      | Low-quality mpv in its own window, hidden until needed               |

### ARGB visual gotcha

On a 32-bit depth visual, `XBlackPixel` is **fully transparent** (0x00000000) — you get a hole straight through to the desktop. We detect the visual depth and use opaque black `0xFF000000` plus `XSetWindowBackground` for initial backgrounds. Without this, a fresh mpv window briefly shows whatever's behind it before the first frame paints.

### XSync vs. XFlush

`XFlush` queues drawing commands; `XSync` blocks until the server has executed them. On `backup.show()` we do `XMapRaised` + `XSync` so that the next mpv frame is guaranteed to land on top. Missing this produces a 1-frame glimpse of the main mpv state before the backup takes over.

## Services (`src/services/`)

### `api.rs`

`reqwest::blocking::Client` with a cookie jar and 8 s timeout, behind a `OnceLock`. Functions:
- `login(email, password) -> Result<User, String>`
- `register(username, email, password) -> Result<User, String>`
- `fetch_me() -> Result<User, String>`
- `logout() -> Result<(), String>`
- `fetch_channels() -> Result<Vec<ServerChannel>, String>`
- `fetch_tv_state(channel_id)` (rare — main flow is Socket.IO)
- `fetch_user_settings()` / `put_user_settings(s)` (401-safe for anonymous users)
- `fetch_bytes(url)` (avatar downloads)

`ServerChannel` now carries `{id, name, handle, avatar}` — the desktop prefers the server's avatar and handle but falls back to the hardcoded `models::channel::get_channels()` list if empty.

### `websocket.rs`

`rust_socketio` client on its own thread. Exposes a `Sender<ClientCommand>` and a `Receiver<ServerEvent>`. Reconnects on disconnect. On connect, sleeps 150 ms for the namespace handshake, then sends the session-local anonymous pseudo + colour.

### `ytdlp_updater.rs`

- Path: `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp` (or `$HOME/.local/share/…`).
- On spawn: immediate `tick` (download if missing → `yt-dlp -U --update-to stable` → log version diff) → `thread::sleep(6 h)` → repeat.
- `binary_path()` is read by `player.rs` to inject the path into mpv via `script-opts=ytdl_hook-ytdl_path=…`.

### `settings.rs`

Serde JSON to `~/.config/koala-tv/settings.json`. Soft preference: any read/write error is logged and ignored (not critical). On login, server settings win over local; on change, we push via `api::put_user_settings`.

### `pseudo.rs`

Same lists as the web client (French animals / fruits / vegetables) + vibrant HSL colour generator. Cached in a `OnceLock<String>` so the pseudo is stable for the whole session.

### `emoji_data.rs`

Emoji categories + codepoints; tiles loaded on demand from `assets/emoji-png/<codepoint>.png` (Apple emoji set).

## Event trace: channel switch

```
SidebarView::on_click(index)
   → emit ChannelSelected("squeezie", "Squeezie", "Squeezie")
      ↓
AppView::subscribe(ChannelSelected)
   - clear chat
   - send ClientCommand::SwitchChannel("squeezie")
   - send ClientCommand::ChatChannelChanged("squeezie")
      ↓
websocket thread
   - emit "tv:switchChannel" "squeezie"
   - emit "chat:channelChanged" "squeezie"
      ↓
server replies:
   - "tv:state" { videoId, seekTo, ... }
   - "chat:history" [ ... ]
      ↓
event_rx receives ServerEvent::TvState + ServerEvent::ChatHistory
   ↓
PlayerView::load_state(&state, cx)
   - detects channel change
   - attach_backup_quality(videoId, cx)              // start backup mpv
   - pending_main_load = Some((url, seek))           // main deferred
   - switch_arm_at = now                             // grace-period timer
      ↓
poll_first_frame_ready() on backup
   → backup.show()          (XMapRaised + XSync)
   → kick main loadfile
      ↓
main mpv VIDEO_RECONFIG
   → audio crossfade 120 ms main ← backup
   → backup.move_offscreen()
   → if drift > 300 ms: main.seek(backup_pos)
```

## Cargo dependencies

| Crate                | Role                                              |
| -------------------- | ------------------------------------------------- |
| `gpui` (git, Zed)    | UI framework                                      |
| `gpui_platform`      | X11 / Wayland backend                             |
| `gpui-component`     | Input, Slider, Popover                            |
| `libmpv2 = "5"`      | Video playback (C bindings to libmpv)             |
| `reqwest = "0.12"`   | HTTP (blocking, cookies, rustls-tls)              |
| `rust_socketio`      | Socket.IO client                                  |
| `resvg`/`tiny-skia`  | SVG rasterization for icons                       |
| `png`/`image`        | Avatar decoding                                   |
| `x11-dl` (Linux)     | Xlib bindings for native windows                  |

## Running

```bash
# from desktop/
cargo run              # dev build
cargo run --release    # slower build, much lower CPU during playback
```

System requirements on Linux:
- libmpv2 dev headers: `sudo apt install libmpv-dev`
- X11 libs: `libx11-dev` (usually already present with a desktop)

# Desktop app reference

Rust (edition 2021) + [GPUI](https://www.gpui.rs) (Zed's UI framework) driving **two external mpv subprocesses** controlled over JSON IPC (`--input-ipc-server` + Unix socket). Linux-first (X11) ; the codebase compiles on Windows and macOS via `cfg(target_os = "linux")` gates but the in-window mpv embed and native overlays (popup menus, tooltip, badge, backup player) are Linux-only for now. See [CROSS_PLATFORM.md](CROSS_PLATFORM.md) for the per-platform status and roadmap.

## Why a native app?

- Browser YouTube iframes forbid some videos (copyright / creator setting). mpv + yt-dlp plays any public video, so the TV never "skips" on a non-embeddable.
- Lower overhead than Chromium : ~230 MB RSS and ~58 threads for 1080p playback with a live backup stream, vs. a gigabyte for Chromium.
- Full control over subtitle tracks, audio tracks, quality selection.

## Entry

`src/main.rs` :

1. Initialises the structured `tracing` logger (`services/logger.rs` — file appender to `$XDG_DATA_HOME/KoalaTV/logs/`).
2. Spawns the yt-dlp auto-updater daemon thread (downloads / `-U` / sleep 6 h / repeat). Env knobs : `YTDLP_UPDATE_INTERVAL_SECS`, `YTDLP_SPAWN_TIMEOUT_SECS`, `YTDLP_DOWNLOAD_URL`.
3. `gpui_platform::application().run(...)` — X11/Wayland backend init.
4. Forces `ThemeMode::Dark` so all gpui-component widgets use a dark palette.
5. Opens a 1280×800 window titled "Koala TV" and mounts `AppView` wrapped in `gpui_component::Root`.

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

## The mpv embed (`src/views/player.rs` + split sub-modules `views/player/{controls,lifecycle,poll,render,x11_errors}.rs`)

The most delicate piece. An `MpvIpcClient` from `services/mpv_ipc.rs` spawns mpv as an external subprocess (`mpv --input-ipc-server=<socket>` + its flags) and communicates over JSON on a Unix domain socket. The subprocess renders directly into an X11 child window passed via the `--wid=<xid>` CLI flag. No libmpv linkage — everything is JSON IPC + CLI flags.

The IPC wrapper exposes a `libmpv2`-shaped API (`spawn / set_property / get_property / command / wait_event`) with a dedicated reader thread that demultiplexes async events from command replies (keyed by `request_id`). Measured round-trip latency : p99 < 0.1 ms.

Socket paths live under `$XDG_RUNTIME_DIR` (fallback `$TMPDIR` → `/tmp`) as `koala-mpv-<pid>-<nanos>.sock`. On `MpvIpcClient::Drop` : SIGTERM → 500 ms grace → SIGKILL → blocking `wait()` → socket file removed.

### Spawn flags (CLI, not init)

All properties are passed to mpv at spawn time via CLI flags — easy to inspect with `ps auxf`. For the main mpv :

```
mpv --idle=yes \
    --input-ipc-server=<socket> \
    --no-terminal --no-input-default-bindings \
    --wid=<child_window_xid> \
    --ytdl=yes --osc=no --input-vo-keyboard=no \
    --force-window=yes --keep-open=no --hwdec=auto-safe \
    --cache=yes --vo=gpu-next --cursor-autohide=no \
    --volume=100 --prefetch-playlist=yes \
    --cache-secs=30 --demuxer-readahead-secs=15 \
    --demuxer-max-bytes=200MiB --demuxer-max-back-bytes=50MiB \
    --stream-lavf-o=reconnect=1,reconnect_streamed=1,reconnect_delay_max=5 \
    --cache-pause-initial=no --cache-pause-wait=1.0 \
    --hr-seek=no --network-timeout=10 --video-latency-hacks=yes \
    --profile=fast --scale=bilinear --dscale=bilinear --cscale=bilinear \
    --sigmoid-upscaling=no --correct-downscaling=no --linear-downscaling=no \
    --deband=no --video-sync=audio --interpolation=no \
    --sub-visibility=no --sub-auto=all \
    --ytdl-raw-options=sub-langs=all,write-auto-subs=,write-subs= \
    --ytdl-format=bestvideo[height<=1080][vcodec!*=av01]+bestaudio/... \
    --script-opts=ytdl_hook-ytdl_path=<yt-dlp path>
```

Backup mpv (LQ 360p) adds `--mute=yes` and a lighter `--cache-secs=10` + `--demuxer-max-bytes=40MiB` to keep RAM low while frozen. Quality/audio/subtitle track changes at runtime go through `set_property` over IPC.

### Pre-resolved URL fast path

When the server's url-resolver has a fresh Redis cache entry for the channel's current videoId, `tv:state` carries `resolvedUrl` (HQ progressive / HLS manifest) and `resolvedUrlLq`. `views/player/lifecycle.rs::load_state` picks the resolved URL over the `youtube.com/watch?v=` fallback, toggles `ytdl=no` on mpv for that loadfile, and passes the URL directly. Skips the ~200-800 ms yt-dlp subprocess spawn entirely ; cold-zap first-frame drops from ~300 ms to ~100 ms. Staleness gate : `TvState::resolved_url_is_fresh()` rejects anything > 5 h old (env `KOALA_RESOLVED_URL_MAX_AGE_SECS`).

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

A small LRU of "frozen" backup mpv subprocesses (one per visited channel) is kept in `memory_cache.rs`. Capacity is user-configurable (0 = disabled, default 2). Zapping back to a recent channel reveals its frozen backup instantly while the main reloads.

### Frame snapshot cache (favorites only)

Client-only feature in `services/frame_cache.rs`. For every favorite channel, the client pre-fetches `https://img.youtube.com/vi/<videoId>/maxresdefault.jpg` (fallback `hqdefault.jpg`) and holds it as `Arc<gpui::Image>` in `AppView::frame_cache`. On click, `subscriptions::channel_click` passes the cached image to `PlayerView::show_snapshot` ; the render path paints it via a GPUI `img()` element in the video area while mpv is held off-screen via `apply_geometry(-10000, …)`. The poll loop clears the snapshot the moment either mpv fires `VideoReconfig` or the backup becomes the visible surface (`pending_backup_reveal`).

Trade-off : the user sees an immediate visual change to the target channel (the video's cover thumbnail, not the exact frame-at-seekTo) instead of the previous channel's frozen last frame — perceived zap = 0 ms. Scoped to favorites because RAM scales linearly (~150 KB decoded × N favorites). Triggers :
- **Boot** : `background_tasks::state_prefetch` tv:state replies arm `fetch_snapshot` for each favorite.
- **Session** : `dispatch.rs` tv:sync hook detects videoId changes on favorites → refetch.
- **Add favorite** : `sync_frame_cache_to_favorites` fetches immediately (videoId already known in `last_state_per_channel`).
- **Remove favorite** : `FrameCache::evict_non_favorites` drops the Arc.

Zero server involvement ; client talks direct to `img.youtube.com` (same host mpv already resolves).

### Quality / audio / subtitle menus

Not gpui-component popovers — they're native X11 windows (`src/views/popup_menu.rs`). They're raised above mpv with their own event loop (`XButtonPress`, `PointerMotionMask`, `ExposureMask`). Hover / click / scroll handled natively. Antialiased text via Xft. They emit `MenuEvent::Selected { kind, index }` back to the Rust side.

This is done natively because GPUI's popovers would render under the mpv X11 window (mpv has its own compositing path).

### Subtitle language filter

YouTube's auto-translated subtitles expose ~200 languages. We pass `sub-langs=all` to yt-dlp, but in the UI we show only 5 common languages (fr, en, de, es, it) by default with a "Plus de langues" option to expand.

## Module split

Two big files (`app.rs`, `views/player.rs`) were split into sub-modules to hit a LOC target from the code-quality audit. Same module tree — child files inherit private-field access on the parent struct via `pub(super)`.

| Parent | Children (`<parent>/<name>.rs`) |
|---|---|
| `app.rs` (300 LOC) | `fps`, `helpers`, `modals`, `dispatch`, `subscriptions`, `background_tasks`, `render` |
| `views/player.rs` (~690 LOC) | `controls`, `lifecycle`, `poll`, `render`, `x11_errors` (+ sibling `player_util.rs`, `player_widgets.rs`) |

## Native X11 bits (all `cfg(target_os = "linux")`)

| File | Role |
|---|---|
| `views/tooltip.rs` | Override-redirect tooltip window with antialiased text (Xft) |
| `views/popup_menu.rs` | Native popup menus (quality / audio / subtitles) |
| `views/channel_badge.rs` | Top-left overlay : avatar · name · ⭐ (if favorite) |
| `views/loading_overlay.rs` | Black overlay with "Chargement…" during channel switch |
| `views/backup_player.rs` | Low-quality mpv subprocess in its own X11 child window, kept off-screen until needed. `move_offscreen()` only (never `XUnmap` — would stall the decoder). |

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

`rust_socketio` client on its own thread. Exposes a `Sender<ClientCommand>` and a `Receiver<ServerEvent>`. Reconnects on disconnect. On connect, sleeps `KOALA_WS_HANDSHAKE_DELAY_MS` (default 150 ms) for the namespace handshake, then sends the session-local anonymous pseudo + colour. Reconnect cooldown : `KOALA_WS_RECONNECT_COOLDOWN_SECS` (default 1). Connect backoff on error : `KOALA_WS_CONNECT_BACKOFF_SECS` (default 3). Command-loop poll tick : `KOALA_WS_CMD_TICK_SECS` (default 1).

### `mpv_ipc.rs`

Wrapper around an external mpv subprocess controlled by JSON IPC. `MpvIpcClient::spawn(&flags)` → `mpv --input-ipc-server=<sock> <flags>`. A reader thread demultiplexes stdout into event stream (async) + command replies (request_id-keyed). `Drop` sends SIGTERM → 500 ms → SIGKILL and removes the socket file. POC standalone binary : `cargo run --release --bin mpv_ipc_poc`. Tests : `cargo test --release -- --ignored mpv_ipc`.

### `mpv_checked.rs`

`mpv_try!` / `emit_try!` macros. Every IPC call site wraps the result and emits a `tracing::warn!` on failure with `op="<what>"` + `ctx="<args>"`. Silent failures used to mask bugs ; now they surface in `$XDG_DATA_HOME/KoalaTV/logs/`.

### `frame_cache.rs`

See [Frame snapshot cache](#frame-snapshot-cache-favorites-only).

### `ytdlp_updater.rs`

- Path : `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp` (or `$HOME/.local/share/KoalaTV/bin/…`).
- On spawn : immediate tick (download if missing → `yt-dlp -U --update-to stable` → `tracing::info` with version diff) → `thread::sleep(update_interval())` → repeat.
- `binary_path()` is read by `player.rs` and injected into mpv's spawn flags via `--script-opts=ytdl_hook-ytdl_path=…`.
- Env : `YTDLP_UPDATE_INTERVAL_SECS` (default 21 600), `YTDLP_SPAWN_TIMEOUT_SECS` (120), `YTDLP_DOWNLOAD_URL` (GitHub latest).

### `logger.rs`

Structured `tracing` subscriber with file appender. Logs rotate daily in `$XDG_DATA_HOME/KoalaTV/logs/desktop.log.YYYY-MM-DD`, kept for `LOG_KEEP_DAYS` (14). Also batch-ships levels ≥ `warn` to the server `/api/logs` endpoint for centralised log capture.

### `state_cache.rs`

Persists `AppView::last_state_per_channel` to disk every 30 s so a fresh boot has a warm cache for the optimistic instant-zap path. Location : `$XDG_DATA_HOME/KoalaTV/state_cache.json`.

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

| Crate | Role |
|---|---|
| `gpui` (git, Zed) | UI framework |
| `gpui_platform` | X11 / Wayland backend |
| `gpui-component` | Input, Slider, Popover |
| `reqwest = "0.12"` | HTTP (blocking, cookies, rustls-tls) |
| `rust_socketio` | Socket.IO client |
| `serde_json` | mpv IPC wire format |
| `tracing` / `tracing-subscriber` / `tracing-appender` | structured logging |
| `resvg` / `tiny-skia` | SVG rasterization for icons |
| `png` / `image` | Avatar + snapshot decoding |
| `x11-dl` (Linux) | Xlib bindings for native windows |

No `libmpv2` — mpv runs as an external subprocess and is reached over JSON IPC on a Unix socket. The only mpv-related build-time dep is `serde_json`.

## Running

```bash
# from desktop/
cargo run              # dev build
cargo run --release    # slower build, much lower CPU during playback
```

System requirements on Linux :
- `mpv` runtime binary (no headers needed) — `sudo apt install mpv` or distro equivalent.
- X11 libs : `libx11-dev` (usually already present with a desktop).
- yt-dlp does NOT need to be pre-installed ; the app auto-downloads and self-updates its own copy under `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp` on first launch.

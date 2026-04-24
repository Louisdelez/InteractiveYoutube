# Web client reference

Vite + React 19 + Socket.IO client. Optional Tauri shell for a native desktop window that embeds a proper YouTube webview (see `client/src-tauri/`).

> Not to be confused with the **main desktop app** in `/desktop` (Rust + GPUI + two external mpv subprocesses via JSON IPC). The Tauri shell is a thin wrapper around this same React code ; the Rust desktop app is a full rewrite with its own mpv IPC pipeline. See [DESKTOP.md](DESKTOP.md).

## Entry

`index.html` → `src/main.jsx` branches:

```js
if (isTauri()) render(<TauriApp />)   // native YouTube webview overlay
else           render(<App />)        // browser iframe / fallback
```

`src/App.jsx` is the browser root. Three-column layout:

```
┌──────────────────────────────────────────────────────────────┐
│ top bar (36 px): title · search · chat toggle · user/login   │
├────────┬──────────────────────────────────────┬──────────────┤
│ sidebar│   player (iframe / fallback)         │   chat       │
│ 56 px  │                                      │   340 px     │
└────────┴──────────────────────────────────────┴──────────────┘
```

On `!isConnected`, a top banner "Reconnexion en cours…" shows.

## Components

| Component            | Purpose                                                                                 |
| -------------------- | --------------------------------------------------------------------------------------- |
| `Player`             | Hosts the sync hook, picks render path (iframe / fallback / Tauri), wires controls      |
| `PlayerFallback`     | Shown on web when `tvState.embeddable === false` — YouTube deep-link + app download     |
| `TauriPlayer`        | Creates / navigates / resizes the native YouTube webview (Tauri only)                   |
| `ChannelSidebar`     | Fetches `/api/tv/channels`, filters by search, keyboard navigation (↑↓ Enter)           |
| `Chat`               | Virtualized message list (`@tanstack/react-virtual`), input, emoji picker               |
| `ChatMessage`        | Memoized single message (time · username · color · text · star if registered)          |
| `VolumeControl`      | Mute toggle + hover-reveal slider                                                       |
| `CaptionsControl`    | Loads YouTube captions module on demand; dropdown of available languages                |
| `ViewerCount`        | Eye icon + live count                                                                   |
| `AuthModal`          | Login / register tabs, email + password + optional username                             |

Styling: one `.css` file per component, dark theme (`#0e0e10` bg, `#efeff1` text, `#9b59b6` accent). Responsive via `@media (max-width: 768px)`.

## Hooks

### `useTvSync(channelId)`

The core state machine. Returns `{tvState, isLoading, onPlayerReady, onVideoEnd, onVideoError, clockOffset}`.

**Clock offset** (5-ping median):
```
offset = median(serverTime - clientTime - rtt/2) over 5 samples
```
Recomputed on every reconnect.

**Drift correction** on `tv:sync`:
```js
const expected = state.seekTo + (now - (state.serverTime - offset)) / 1000;
if (Math.abs(player.getCurrentTime() - expected) > 4) {
  player.seekTo(expected, true);
}
```

**Channel switch**: emits `tv:switchChannel`, fetches `/api/tv/state?channel=…` (AbortController for cleanup). On `tv:state`, updates local state. On `tv:refreshed`, refetches.

**End-of-video**: `onVideoEnd` refetches — server has already advanced the playhead, so the response contains the new video.

### `useChat(channelId)`

Manages a rolling buffer of 300 messages (oldest trimmed first).

- **Buffering + rAF flush**: incoming `chat:batch` / `chat:message` events push into `pendingRef`; a `requestAnimationFrame` coalesces them into a single state update. Prevents >60 renders/s during bursts.
- Listens to `chat:history` (full replace on channel change), `chat:cleared` (reset to `[]`), `viewers:count`, `chat:error`.
- Emits `chat:setAnonymousName` on connect with the session-scoped pseudo + color.
- Returns `{messages, viewerCount, sendMessage}`.

### `useAuth()`

Fetches `/api/auth/me` on mount. `login` / `register` / `logout` all call the respective `/api/auth/*` endpoint and then `window.location.reload()` — simpler than re-handshaking the Socket.IO connection with new cookies.

### `useSocket()`

Calls `socket.connect()` on mount (socket is configured with `autoConnect: false`). Tracks `isConnected` based on `connect` / `disconnect` events.

## Services

### `src/services/api.js`

Tiny fetch wrapper with `credentials: 'include'` and JSON handling. On Tauri, `BASE_URL` is pulled from `localStorage.iyt-server-url` so the packaged app can point at a remote server. On web, `BASE_URL = ''` (same-origin via Vite proxy in dev / nginx in prod).

```js
export const api = {
  get:  (path, opts) => request('GET',  path, null, opts),
  post: (path, body, opts) => request('POST', path, body, opts),
};
```

### `src/services/socket.js`

Singleton Socket.IO client, `autoConnect: false`, `withCredentials: true`. Same BASE_URL logic as `api.js`.

### `src/services/platform.js`

`isTauri()` checks `Boolean(window.__TAURI_INTERNALS__)`. Used in:
- `main.jsx` — root component selection
- `socket.js` / `api.js` — server URL resolution
- `Player.jsx` — render path selection
- `PlayerFallback.jsx` — skip auto-advance timer (Tauri handles it natively)

### `src/services/pseudoGenerator.js`

Picks a random French animal / fruit / vegetable name and a vibrant HSL colour. Cached in `sessionStorage` so the pseudo is stable for the whole session.

## Player flow

```
┌───────────┐                    ┌─────────────────┐
│  Player   │  tvState.embeddable │  YouTube iframe │
│  picks    │ ──────── true ────▶│  (react-youtube)│
│  render   │                    └─────────────────┘
│  path     │  tvState.embeddable │  PlayerFallback │
│  based    │ ──────── false ───▶│  (YT link +     │
│  on isTauri                   │   app download) │
│  and       │                    └─────────────────┘
│  embeddable│  isTauri()          │  TauriPlayer    │
└───────────┘ ─────── true ─────▶│  (native webview│
                                 │   overlay)      │
                                 └─────────────────┘
```

Auto-advance on web non-embeddable: a `setTimeout((duration - seekTo) * 1000, onVideoEnd)` drives the playhead forward so chat and sidebar stay in sync even without a real player.

## Fallback UX

When YouTube refuses to embed:
1. Replace the iframe area with `PlayerFallback`.
2. Show title + current timecode (updated every 1 s): `YouTube ▸ 3m42s`.
3. Deep link to `youtube.com/watch?v=<id>&t=<timecode>s` — opens in a new tab at the right second.
4. Fetch `/api/tv/desktop-download` and render a second button to the native app.
5. Sidebar and chat remain fully interactive; when the TV advances to the next (embeddable) video, the iframe returns automatically.

## Build

```bash
npm install           # from client/
npm run dev           # vite on :4501 (proxies /api + /socket.io to :4500)
npm run build         # → client/dist/
npm run preview       # serves the production bundle locally
npm run lint          # ESLint on .jsx/.js
```

`vite.config.js` sets the dev proxy :

```js
server: {
  port: 4501,
  proxy: {
    '/api':       'http://localhost:4500',
    '/socket.io': { target: 'http://localhost:4500', ws: true },
  },
}
```

In production the React bundle is served by nginx from `client/dist/`, and `/api/*` + `/socket.io/*` are proxied to the Node cluster on `:4500`.

## i18n

Strings are externalised to `shared/i18n/fr.json`, consumed by **four parallel lookup helpers** (one per stack) so the same key tree works everywhere :

| Stack | Helper | API |
|---|---|---|
| Web (this) | `client/src/i18n/index.js` | `t('key')` / `t('key', { name: 'Alice' })` |
| Status app | `status-app/src/i18n.js` | same API, shares the bundle |
| Desktop Rust | `desktop/src/i18n/mod.rs` | `t("key")` → `String` / `t_args("key", &[("name","Alice")])` |
| Node server | `server/i18n/fr.js` | `t('key')` |

Interpolation : `{name}` placeholders are substituted via `value.replace(/\{(\w+)\}/g, …)`. Missing keys log once per key and return the key itself — broken translations are visible in the UI rather than crashing.

Key naming convention : `<scope>.<feature>.<variant>` (examples : `chat.send`, `topbar.search.placeholder`, `status.banner.all_operational`, `date.ago.years_plural`).

Semantic literals stay in code (native-language labels `"Français"`, `"日本語"`, pseudo animal names, proper-noun tab labels `"Emoji"` / `"GIF"` / `"Stickers"`, resolution labels, Tenor brand attribution, stable JSON error identifiers, brand strings).

## Environment variables (Vite)

Any `VITE_*` prefix is exposed to the client at build time.

| Variable | Default | Purpose |
|---|---|---|
| `VITE_API_URL` | `http://localhost:4500` | Server base URL override for packaged builds / staging |
| `VITE_REPO_URL` | `https://github.com/Louisdelez/KoalaTV` | GitHub link in the top bar + about page |
| `VITE_REPO_SLUG` | `Louisdelez/KoalaTV` | Download-page release API path |
| `VITE_STATUS_POLL_MS` | `30000` | StatusPage auto-refresh interval |
| `VITE_YT_SEEK_DELAY_MS` | `3000` | TauriApp : grace before issuing seek after loadVideo |
| `VITE_YT_POS_SYNC_MS` | `300` | TauriApp : overlay ↔ webview position-sync tick |
| `VITE_GIF_SEARCH_DEBOUNCE_MS` | `300` | ChatPicker Tenor search debounce |
| `VITE_DESKTOP_DOWNLOAD_URL` | GitHub releases | Fallback for PlayerFallback app-install button |
| `VITE_BRAND_HOME_URL` | `https://koalatv.com` | status-app footer brand link |

## Tauri shell (optional)

Located at `client/src-tauri/`. Provides Rust-invokable commands:

- `create_youtube_webview({videoId, x, y, width, height})` — spawn a native YouTube window overlaid on the player area.
- `youtube_navigate({videoId})` — load a new video in the existing window.
- `youtube_seek({seconds})`, `youtube_resize({x, y, w, h})`, `youtube_destroy()`, `youtube_show()`, `youtube_hide()`.

`TauriApp.jsx` drives this from React: a `ResizeObserver` + 300 ms interval keeps the native window aligned with the React player area as the layout changes.

This shell is kept because it's a simpler deliverable than the GPUI rewrite for users who only need the "play non-embeddable videos" feature. The `/desktop` app (Rust + GPUI + 2× mpv subprocess via IPC) supersedes it for the full experience : dual-mpv zero-cut playback, memory cache for instant zap on recent channels, per-favorite frame snapshot cache, server-pre-resolved googlevideo URLs for ~100 ms cold-zap first frame.

# Server reference

Node.js 20 + Express + Socket.IO, clustered behind PM2, backed by PostgreSQL (users/settings) and Redis (chat history, rate limits, viewer sets, Socket.IO adapter).

## Boot sequence

`server/index.js` starts in this order:

1. `initDB()` — ensures the `users` table and indexes exist (see [Database](#database)).
2. `ytdlpUpdater.start()` — downloads `./bin/yt-dlp` if missing, self-updates, then ticks every 6 h (non-blocking).
3. `initAllPlaylists()` — loads `server/data/playlist-*.json` from disk or builds from the YouTube API on first run.
4. `setupSocketIO(server)` — binds the Redis adapter, registers handlers.
5. `startCronJobs()` — schedules the 3 am daily refresh and the 30-min RSS poll.
6. `server.listen(PORT)`.

Graceful shutdown (SIGTERM/SIGINT): stop Socket.IO → close HTTP → close DB pool → `process.exit(0)`. Force-kill after 10 s (`kill_timeout` in `ecosystem.config.js`).

## HTTP API

All JSON. Authenticated endpoints expect a `token` cookie (HttpOnly, 7-day JWT).

### `/api/auth`

| Method | Path         | Auth | Body / Query                   | Response                                     | Rate limit      |
| ------ | ------------ | ---- | ------------------------------ | -------------------------------------------- | --------------- |
| POST   | `/register`  | —    | `{username, email, password}`  | `{user: {id, username, color}}` + cookie     | 10 / hour / IP  |
| POST   | `/login`     | —    | `{email, password}`            | `{user: {id, username, color}}` + cookie     | 5 / 15 min / IP |
| POST   | `/logout`    | —    | —                              | `{ok: true}`, clears cookie                  | —               |
| GET    | `/me`        | opt. | —                              | `{user}` or 401                              | —               |

Validation:
- username: 3–20 chars, alphanumeric + `-` `_`
- password: ≥ 6 chars
- Twitch-style HSL color auto-assigned at register.

### `/api/tv`

| Method | Path                  | Auth | Query / Response                                                    |
| ------ | --------------------- | ---- | ------------------------------------------------------------------- |
| GET    | `/state`              | —    | `?channel=<id>` → full `TvState` or 503                             |
| GET    | `/channels`           | —    | `[{id, name, handle, avatar}]` — used by web + desktop sidebars     |
| GET    | `/desktop-download`   | —    | `{url}` — configured by `DESKTOP_DOWNLOAD_URL` env var              |

`TvState` shape (always in seconds):

```ts
{
  videoId: string
  title: string
  videoIndex: number    // -1 when priority
  seekTo: number        // current in-video offset
  duration: number
  embeddable: boolean
  serverTime: number    // epoch ms at the moment of computation
  totalVideos: number
  channelId: string
  isPriority: boolean
  nextVideoId: string
  nextTitle: string
  nextDuration: number
}
```

### `/api/user`

| Method | Path         | Auth | Body                                                   | Response                      |
| ------ | ------------ | ---- | ------------------------------------------------------ | ----------------------------- |
| GET    | `/settings`  | ✓    | —                                                      | `{settings: {...}}`           |
| PUT    | `/settings`  | ✓    | `{settings: {memory_capacity?, favorites?: string[]}}` | `{ok: true, settings}`        |

Validated: `memory_capacity` 0–5, `favorites` at most 50 items.

### Infrastructure endpoints

| Method | Path        | Response                                    |
| ------ | ----------- | ------------------------------------------- |
| GET    | `/health`   | `{status: 'ok', uptime: <seconds>}`         |
| GET    | `/metrics`  | Prometheus text format                      |

## Socket.IO protocol

Handshake: cookies are read (JWT verified → logged-in; otherwise anonymous). Max 10 connections per IP per hour (tracked in Redis).

On connect, the client joins `channel:<default>` and receives `tv:state` for that channel and the chat history.

### Client → server

| Event                       | Payload                   | Effect                                                    |
| --------------------------- | ------------------------- | --------------------------------------------------------- |
| `tv:ping`                   | `{ clientTime }`          | echoes `tv:pong` (clock sync)                             |
| `tv:switchChannel`          | `<channelId>`             | debounced 400 ms, leaves old room, joins new, sends state |
| `tv:requestState`           | —                         | re-emits `tv:state` for current channel                   |
| `tv:videoError`             | `{ videoId, error }`      | logged                                                    |
| `chat:message`              | `{ text }`                | rate-limited; appended to Redis list; batched out         |
| `chat:channelChanged`       | `<channelId>`             | emits `chat:history` for the new channel                  |
| `chat:setAnonymousName`     | `{ name, color }`         | pseudo used for outgoing messages when not logged in      |

### Server → client

| Event                | Payload                                                     | When                                   |
| -------------------- | ----------------------------------------------------------- | -------------------------------------- |
| `tv:state`           | `TvState`                                                   | connect / switch / request / refresh   |
| `tv:sync`            | `TvState`                                                   | every 15 s (volatile broadcast)        |
| `tv:refreshed`       | —                                                           | after per-channel refresh — clients refetch |
| `tv:newRelease`      | `{ videos: [{title, videoId}] }`                            | RSS poll finds new uploads             |
| `tv:pong`            | `{ clientTime, serverTime }`                                | reply to `tv:ping`                     |
| `chat:history`       | `[msg, …]` — full history (last 200)                        | on connect / channel change            |
| `chat:batch`         | `[msg, …]` — new messages since last tick (150 ms)          | volatile, per channel                  |
| `chat:cleared`       | —                                                           | daily 3 am wipe                        |
| `chat:error`         | `{ error }`                                                 | rate-limited, validation errors        |
| `viewers:count`      | `{ count }`                                                 | on join/leave, reconciled every 60 s   |

## Services

### `services/playlist.js`

Builds, merges, and persists the per-channel playlist.

- **Seeded shuffle** (Mulberry32): deterministic shuffle per channel, seed stored in the JSON for reproducibility.
- **`prefixSums`** (Float64Array): cumulative durations for O(log N) seek lookup.
- **`buildPlaylist(channelId, channel)`**: normal channels fetch all uploads + dedupe + shuffle; `ordered` channels use `fixedVideoIds` or `youtubePlaylists` verbatim.
- **`refreshPlaylist(channelId)`**: fetches fresh IDs, filters against the known set, appends only truly-new videos via `mergePlaylistPreservingTimecode`.
- **`addNewVideos(channelId, videos)`** (used by RSS poll): same merge path.
- **`mergePlaylistPreservingTimecode(oldState, newVideos)`**: appends + rebases `tvStartedAt` so `(now - tvStartedAt) mod totalDuration` is preserved.
- **Persistence**: atomic `.tmp` → `rename`. Loaded back on boot via `loadFromDisk`.
- **Refresh lock**: per-channel in-flight promise map so concurrent calls for the same channel reuse the same work.

### `services/tv.js`

Computes the current `TvState` from the playlist + priority queue.

- 1 s in-memory cache per channel (covers the 15 s sync broadcast + all HTTP `GET /state` during that second).
- Normal rotation: binary search on prefix sums.
- Priority queue: if non-empty on a rotation boundary, play `queue[0]` with `seekTo=0, isPriority=true`; after it finishes, shift and resume rotation at the natural point.
- `queuePriorityVideo(channelId, video)` is called by the RSS poller when a new upload is detected.

### `services/youtube.js`

Wraps the `googleapis` client (YouTube Data API v3).

- `fetchAllVideoIds(channelId)`: paginates the channel's uploads playlist (50 items/page).
- `fetchOrderedVideoIds(playlistIds)`: concatenates items from multiple playlists in order.
- `fetchVideoDetails(ids, {skipShortsFilter, skipLiveFilter})`: batches 50 IDs/call; parses ISO 8601 durations; filters `duration=0`, live streams/replays (unless opted out), and Shorts.
- **Shorts detection** without quota: HEAD request to `youtube.com/shorts/<id>` — 200 means it's a Short. Cached to `server/data/shorts-cache.json`. Jittered 0–200 ms, concurrency cap 20.

### `services/rss.js`

Polls `https://www.youtube.com/feeds/videos.xml?channel_id=<id>` every 30 min. Parses with `fast-xml-parser`, diffs against known video IDs, returns only new ones.

### `services/ytdlp-updater.js`

- Binary lives under `<repo>/bin/yt-dlp` (gitignored).
- `ensureBinary()`: downloads from `github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp` if missing (follows up to 5 redirects, atomic write).
- `selfUpdate()`: `yt-dlp -U --update-to stable`, logs version diff.
- Tick loop: immediate + every 6 h.
- `server/scripts/ytdlp-audit.sh` prefers this binary via `$YTDLP_BIN` so dev and prod use the same version.

### `services/redis.js`

Three ioredis clients: main, pub, sub. Retries up to 3 ×, exp-backoff capped at 3 s. Main client is used for chat history lists, rate-limit ZSETs, viewer sets, IP counters.

### `services/logger.js` / `services/metrics.js`

- `pino` JSON logger. Pretty-printed in dev, structured in prod. Redacts `req.headers.cookie`, `authorization`, password fields. Base metadata `{svc: 'iy-server'}`.
- Prometheus metrics (`prom-client`):
  - `iy_viewers_per_channel` (gauge)
  - `iy_socket_connections_total` (counter)
  - `iy_chat_messages_total{channel}` (counter)
  - `iy_tv_sync_broadcast_duration_ms` (histogram)
  - `iy_auth_attempts_total{kind,status}` (counter)
  - default Node.js metrics prefixed `iy_`.

## Chat

`server/socket/chat.js`:

- **Sanitization**: strips C0/C1 control chars, zero-width chars, BOM, RTL overrides.
- **Unicode-safe clamp**: 500 codepoints max (preserves emoji surrogate pairs).
- **Rate limit**: 5 messages / 5 s, tracked in a Redis ZSET keyed by `u:<userId>` (logged-in) or `ip:<ip>` (anonymous).
- **History**: Redis list `chat:history:<channelId>`, capped at 200 via `LTRIM -200 -1`.
- **Batching**: messages pushed into an in-memory map per channel, flushed every 150 ms as a volatile `chat:batch` emit.
- **Anonymous identity**: in-memory Maps keyed by socket ID for name + color; cleared on disconnect.
- **`clearAllChatHistory(io)`**: SCAN `chat:history:*` + DEL, then broadcast `chat:cleared`. Called from the 3 am cron.

## Cron

`server/cron/refresh.js`:

- **Daily** (`DAILY_REFRESH_CRON = '0 3 * * *'`):
  1. `bucket = channels.filter((_, i) => i % 7 === new Date().getDay())` — one seventh of the channels.
  2. For each: `refreshPlaylist(id)` + `tv:refreshed` broadcast.
  3. `clearAllChatHistory(io)`.
  4. `setTimeout(() => process.exit(1), 2000)` — PM2 respawns (also works with nodemon in dev).
- **RSS poll** (every 30 min, `setInterval`):
  - Normal channels: fetch RSS → diff → `fetchVideoDetails` → `queuePriorityVideo` + `addNewVideos` + `tv:newRelease`.
  - Ordered channels (Popcorn): polls a curated YouTube channel for new episodes, filters by title ("POPCORN") and duration (≥ 1h30), appends at the end (keeps order).

## Database

PostgreSQL, single `users` table:

```sql
CREATE TABLE users (
  id            SERIAL PRIMARY KEY,
  username      TEXT UNIQUE NOT NULL,
  email         TEXT UNIQUE NOT NULL,
  password_hash TEXT NOT NULL,
  color         TEXT NOT NULL DEFAULT '#1E90FF',
  settings      JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at    TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX idx_users_email    ON users(email);
CREATE INDEX idx_users_username ON users(username);
```

Pool: 20 connections, 30 s idle, 5 s connect timeout. `settings` is JSONB: `{memory_capacity, favorites}`.

## Configuration

All from env (`.env` at repo root) or sensible defaults. See `.env.example`.

| Variable                  | Required | Default / Notes                                                                |
| ------------------------- | -------- | ------------------------------------------------------------------------------ |
| `YOUTUBE_API_KEY`         | ✓        | Google Cloud YouTube Data API v3                                               |
| `JWT_SECRET`              | ✓        | 32+ byte random hex                                                            |
| `PORT`                    |          | 4500                                                                           |
| `CLIENT_ORIGIN`           |          | `http://localhost:4501` — CORS allow-list (+ `tauri://localhost`)              |
| `NODE_ENV`                |          | `development`                                                                  |
| `REDIS_URL`               |          | `redis://localhost:6379`                                                       |
| `DATABASE_URL`            |          | `postgresql://interactiveyoutube:interactiveyoutube@localhost:5432/…`          |
| `DESKTOP_DOWNLOAD_URL`    |          | GitHub releases URL — shown in the web fallback when a video isn't embeddable |

## Channels

Defined in `server/config.js` as `CHANNELS`. Each entry:

```js
{
  id: 'amixem',                              // slug
  name: 'Amixem',                            // display name
  handle: 'Amixem',                          // YouTube @handle
  avatar: 'https://yt3.googleusercontent…', // s160 thumbnail URL
  youtubeChannelIds: ['UCgvqvBoSHB1ct…'],   // merge source(s)
  // optional:
  ordered: true,                             // disables shuffle
  fixedVideoIds: ['rPC5dMVmqtw', …],         // hand-picked (Noob)
  youtubePlaylists: ['PLgpA18kDVMTi…', …],  // ordered playlists (Popcorn)
  extraPlaylists: [...],                     // extra playlists merged in (EGO)
  includeLives: true,                        // keep live replays (Legend)
}
```

Adding a channel: add an entry → nodemon (dev) or `pm2 reload` (prod) picks it up → the web sidebar re-fetches on next reload, the desktop on next launch.

## Scripts

- `server/scripts/ytdlp-audit.sh` — counts Videos/Shorts/Streams per channel via yt-dlp (no API quota), compares to cache.
- `server/scripts/audit-channels.js` — same audit via the YouTube Data API.
- `server/scripts/scrape-audit.js` — HTML-scraped fallback (no API, no yt-dlp).

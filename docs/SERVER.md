# Server reference

Node.js 20 + Express + Socket.IO backed by PostgreSQL (users/settings) and Redis (chat history, rate limits, viewer sets, Socket.IO adapter, URL resolver cache, BullMQ queue). Runs as **two separate processes** under PM2 :

| Process | Entry point | Responsibilities |
|---|---|---|
| `koala-tv-web` | `server/index.js` | HTTP + Socket.IO only. No cron, no workers. |
| `koala-tv-maint` | `server/workers/index.js` | BullMQ scheduler + daily maintenance + RSS poll + yt-dlp auto-update + URL pre-resolver. |

The split was introduced 2026-04-21 so a worker crash or nodemon restart triggered by `server/data/*.json` writes can't interrupt the web tier, and so the cron schedule survives web restarts (BullMQ persists it in Redis).

## Boot sequences

**Web** (`server/index.js`) — pure HTTP/WS :

1. `initDB()` — ensures the `users` table and indexes.
2. `setupSocketIO(server)` — binds the Redis adapter, registers handlers.
3. `server.listen(PORT)`.

No cron, no yt-dlp updater, no playlist builder in the web process.

**Worker** (`server/workers/index.js`) — orchestration :

1. `initAllPlaylists()` — loads `server/data/playlist-*.json` from disk or builds from the YouTube API on first run. `loadFromDisk` validates `tvStartedAt` / `totalDuration` / non-empty `videos[]` ; malformed JSON triggers a rebuild instead of silently serving `NaN`.
2. `ytdlpUpdater.start()` — downloads `./bin/yt-dlp` if missing, self-updates, ticks every 6 h.
3. `dailyMaintenance.start()` — arms the BullMQ schedulers (daily 3 am refresh + 2h55 warning broadcast) and binds the Worker to process jobs.
4. `rssPoll.start()` — setInterval, 30 min.
5. `urlResolver.start()` — setInterval, 30 min + 5 s initial delay. Pre-resolves googlevideo URLs via yt-dlp for every channel's current video, caches in Redis with 1 h TTL.

Graceful shutdown (SIGTERM/SIGINT) : stop intervals → let BullMQ workers release their locks via natural socket close → `process.exit(0)`. Force-kill after 10 s (`kill_timeout` in `ecosystem.config.js`).

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

`TvState` shape (always in seconds) :

```ts
{
  videoId: string
  title: string
  videoIndex: number    // -1 when priority
  seekTo: number        // current in-video offset
  duration: number
  embeddable: boolean
  publishedAt: string | null
  serverTime: number    // epoch ms at the moment of computation
  totalVideos: number
  channelId: string
  isPriority: boolean
  nextVideoId: string
  nextTitle: string
  nextDuration: number

  // Optional — present when the url-resolver worker has a fresh
  // Redis cache entry for this channel's current videoId. The
  // desktop client passes `resolvedUrl` straight to mpv.loadfile
  // with `ytdl=no`, skipping the ~200-800 ms yt-dlp step on cold
  // zap. Absent → client falls back to `youtube.com/watch?v=<id>`
  // + its normal ytdl_hook.
  resolvedUrl?: string       // HLS manifest, main quality (≤ 720p progressive)
  resolvedUrlLq?: string     // HLS manifest, backup quality (≤ 360p)
  resolvedAt?: number        // UNIX seconds — client treats > 5 h as stale
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

Handshake : cookies are read (JWT verified → logged-in ; otherwise anonymous). Rate-limited to `SOCKET_MAX_CONNECTIONS_PER_IP` (default 30) per `SOCKET_IP_TTL_SECS` (default 600 s), tracked in Redis at `iy:ip:<ip>`. Loopback (`127.0.0.1` / `::1` / `::ffff:127.0.0.1`) is exempt — dev restarts with ungraceful exits (`pkill -9`) don't emit a clean disconnect, so the counter would otherwise climb monotonically and wedge the local socket after ~30 restarts.

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

## Maintenance worker

Lives in the second Node process (`koala-tv-maint`). Every scheduled task is persisted in Redis via BullMQ, so a worker crash or nodemon restart doesn't skip a run and a web restart doesn't touch the schedule.

### Daily maintenance — `server/workers/daily-maintenance.js`

BullMQ schedulers, armed idempotently on every worker boot via `queue.upsertJobScheduler` :

| Scheduler key | Pattern | Job name | Effect |
|---|---|---|---|
| `koala-daily-3am` | `DAILY_REFRESH_CRON` (default `0 3 * * *`) | `daily-maintenance` | 5-step pipeline, see below |
| `koala-daily-2h55-warning` | `DAILY_WARNING_CRON` (default `55 2 * * *`) | `maintenance-warning` | broadcasts `maintenance:warning` 5 min before |

Daily pipeline, with per-step checkpoints in Redis key `maint:ckpt:<jobId>` (TTL 6 h) so a retry after a crash skips already-completed steps :

1. `yt-dlp -U`
2. Refresh 1/7 of the channels (`index % 7 === dayOfWeek`), 60 s timeout each, fail = skip.
3. Redis rate-limit key cleanup (SCAN + DEL expired keys).
4. Clear chat history : SCAN `chat:history:*` + DEL + broadcast `chat:cleared`.
5. Verify empty : re-SCAN + force-DEL any reliquats.

Retry : `attempts: 3`, exponential backoff 60 s. `lockDuration: 10 min` — `refreshPlaylist` can be slow.

### RSS poll — `server/workers/rss-poll.js`

`setInterval(RSS_POLL_INTERVAL_MS)`, default 30 min. Per-channel polymorphic call to `channel.pollRss()` (`NormalChannel` returns RSS diff ; `OrderedPlaylistChannel` like Popcorn polls a curated channel, filters by title + min duration ; `FixedVideoChannel` returns `[]`).

On new video : `addNewVideos()` writes JSON to disk, publishes `koala:playlist-reload` pub/sub (web reloads its in-memory mirror), and for normal channels `koala:priority-video` per video (web enqueues in its priority queue). Socket.IO emits to clients go through `@socket.io/redis-emitter` directly from the worker — the web process isn't in the path.

### URL pre-resolver — `server/workers/url-resolver.js`

`setInterval(URL_RESOLVER_INTERVAL_MS)`, default 30 min, concurrency 2, initial delay 5 s. For each channel :
1. Read current `videoId` via `getTvState()`.
2. Spawn `yt-dlp -g -f <FMT_MAIN>` and `-f <FMT_LQ>` in parallel (`Promise.allSettled`).
3. Cache `{videoId, mainUrl, lqUrl, resolvedAt}` in Redis at `koala:url:<channelId>` with `URL_RESOLVER_CACHE_TTL_SECS` TTL (default 3600 s).

**Event-driven re-resolution** : `getTvState()` detects videoId changes vs the `lastVideoIds` map (auto-advance) and `setImmediate(() => urlResolver.resolveAndCache(...))` — the next `tv:state` / `tv:sync` request serves a warm URL instead of falling through to client-side ytdl_hook. Cache is also invalidated on priority-video injection.

### Observability

From the web (reads Redis keys the worker writes) :

| Endpoint | Access | Purpose |
|---|---|---|
| `GET /metrics` | public | Prometheus, includes `iy_maintenance_{duration_seconds,last_success_ts,last_failure_ts}` |
| `POST /api/admin/maintenance-trigger` | loopback only | Enqueues an immediate daily-maintenance job on BullMQ |
| `GET /api/admin/cron-status` | loopback only | Lists schedulers, waiting/active counts, recent completed + failed jobs |

Optional dead-man switch : set `HEALTHCHECKS_URL` env var and the worker pings it on success (and `/fail` on failure) after each daily run.

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

All env vars come from `.env` at repo root. Required vars are enforced by `server/config.js` at boot — missing ones fail loudly with every missing var listed at once. See `.env.example` for a full template.

### Required

| Variable | Notes |
|---|---|
| `YOUTUBE_API_KEY` | Google Cloud YouTube Data API v3 |
| `JWT_SECRET` | 32+ byte random hex |
| `DATABASE_URL` | `postgresql://user:pass@host:5432/db` |
| `TENOR_API_KEY` | Google Cloud API key with the Tenor API enabled |

### Core runtime (all optional, with defaults)

| Variable | Default | Notes |
|---|---|---|
| `PORT` | `4500` | Web server listen port |
| `CLIENT_ORIGIN` | `http://localhost:4501` | CORS allow-list (+ `tauri://localhost` + `https://tauri.localhost`) |
| `NODE_ENV` | `development` |  |
| `REDIS_URL` | `redis://localhost:6379` |  |
| `SERVER_TZ` | `$TZ` → `Europe/Paris` | Cron timezone |
| `DESKTOP_DOWNLOAD_URL` | GitHub releases URL | Shown in web fallback for non-embeddable videos |

### Timings (env-overridable knobs — defaults preserve pre-refactor behaviour)

| Scope | Variable | Default |
|---|---|---|
| Cron | `DAILY_REFRESH_CRON` | `0 3 * * *` |
| Cron | `DAILY_WARNING_CRON` | `55 2 * * *` |
| Worker | `RSS_POLL_INTERVAL_MS` | `1_800_000` |
| Worker | `MAINT_REFRESH_TIMEOUT_MS` | `60_000` |
| Worker | `MAINT_CKPT_TTL_SECS` | `21_600` (6 h) |
| Worker | `MAINT_JOB_ATTEMPTS` / `MAINT_JOB_BACKOFF_MS` | `3` / `60_000` |
| Worker | `MAINT_LOCK_DURATION_MS` | `600_000` |
| Worker | `MAINT_STATE_{WARNING,RUNNING}_TTL_SECS` | `900` (15 min) |
| Sync | `DRIFT_CORRECTION_INTERVAL_MS` | `15_000` |
| Chat | `CHAT_BATCH_INTERVAL_MS` | `150` |
| Chat | `CHAT_RATE_WINDOW_MS` / `CHAT_RATE_MAX_MESSAGES` | `5_000` / `5` |
| Chat | `CHAT_BUFFER_SIZE` / `CHAT_RATE_LIMIT_MS` | `200` / `1_000` |
| TV | `TV_STATE_CACHE_TTL_MS` | `1_000` |
| Socket | `SOCKET_PING_TIMEOUT_MS` / `SOCKET_PING_INTERVAL_MS` | `20_000` / `25_000` |
| Socket | `SOCKET_CONNECT_TIMEOUT_MS` / `SOCKET_RECONCILE_INTERVAL_MS` | `10_000` / `60_000` |
| Socket | `SOCKET_MAX_HTTP_BUFFER` | `16384` |
| Socket | `SOCKET_MAX_CONNECTIONS_PER_IP` / `SOCKET_IP_TTL_SECS` | `30` / `600` (loopback is exempt) |
| Socket | `SOCKET_SWITCH_DEBOUNCE_MS` | `400` |
| Auth | `AUTH_LOGIN_RATE_WINDOW_MS` / `AUTH_LOGIN_RATE_MAX` | `900_000` / `5` |
| Auth | `AUTH_REGISTER_RATE_WINDOW_MS` / `AUTH_REGISTER_RATE_MAX` | `3_600_000` / `10` |
| Auth | `JWT_COOKIE_MAX_AGE_MS` | `604_800_000` (7 days) |
| Status | `STATUS_COLLECTOR_INTERVAL_MS` | `60_000` |
| Status | `STATUS_CACHE_MS` / `STATUS_HISTORY_DEFAULT_DAYS` | `5_000` / `90` |
| Status | `STATUS_LATENCY_{POSTGRES,REDIS,YOUTUBE,LOKI}_MS` | `150` / `50` / `1500` / `500` |
| Status | `STATUS_{YOUTUBE,LOKI}_TIMEOUT_MS` | `4000` / `2000` |
| Status | `STATUS_CRON_{DOWN,DEGRADED}_HOURS` | `26` / `25` |
| Status | `STATUS_YTDLP_DEGRADED_DAYS` | `14` |
| Status | `STATUS_DISK_{DOWN,DEGRADED}_PCT` | `5` / `15` |
| Status | `STATUS_PRUNE_INTERVAL_MS` / `STATUS_DISK_PATH` | `86_400_000` / `/` |
| Status | `LOKI_URL` / `CRON_LOG_PATH` | `http://localhost:3100/ready` / `<repo>/logs/koala-cron.log` |
| URL resolver | `URL_RESOLVER_INTERVAL_MS` / `URL_RESOLVER_CONCURRENCY` | `1_800_000` / `2` |
| URL resolver | `URL_RESOLVER_CACHE_TTL_SECS` / `URL_RESOLVER_TIMEOUT_MS` | `3_600` / `25_000` |
| URL resolver | `URL_RESOLVER_FMT_MAIN` / `URL_RESOLVER_FMT_LQ` | see `services/url-resolver.js` |
| URL resolver | `URL_RESOLVER_INITIAL_DELAY_MS` | `5_000` |
| yt-dlp | `YTDLP_BIN_DIR` / `YTDLP_BIN_PATH` | `<repo>/bin` / `<BIN_DIR>/yt-dlp` |
| yt-dlp | `YTDLP_DOWNLOAD_URL` | official GitHub latest release |
| yt-dlp | `YTDLP_UPDATE_INTERVAL_MS` / `YTDLP_SPAWN_TIMEOUT_MS` | `21_600_000` / `120_000` |
| yt-dlp | `YTDLP_DOWNLOAD_TIMEOUT_MS` | `60_000` |
| YouTube probe | `YT_SHORTS_{USER_AGENT,JITTER_MAX_MS,HEAD_TIMEOUT_MS,CONCURRENCY}` | see `services/youtube.js` |
| Tenor | `TENOR_API_BASE` / `TENOR_MEDIA_FILTER` / `TENOR_RESULT_LIMIT` | `https://g.tenor.com/v2` / `gif,tinygif,nanogif` / `30` |
| Redis | `REDIS_RETRY_STEP_MS` / `REDIS_RETRY_MAX_MS` / `REDIS_MAX_RETRIES_PER_REQUEST` | `100` / `3000` / `3` |
| Postgres pool | `PG_POOL_MAX` / `PG_POOL_IDLE_MS` / `PG_POOL_CONNECT_MS` | `20` / `30_000` / `5_000` |
| Logs ingest | `LOG_MAX_{MSG_LEN,CTX_BYTES,EVENTS_PER_BATCH}` | `4000` / `8000` / `50` |
| Logs ingest | `LOG_RATE_WINDOW_MS` / `LOG_RATE_MAX` | `60_000` / `120` |
| Cache headers | `STICKER_CACHE_MAX_AGE` / `CLIENT_ASSETS_CACHE_MAX_AGE` | `7d` / `1y` |
| Dead-man switch | `HEALTHCHECKS_URL` | optional; pinged by worker on maintenance success/fail |

Pre-refactor values were baked in as constants. Every knob above keeps the historical literal as its fallback, so an empty `.env` beyond the required 4 vars is a fully valid dev config.

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

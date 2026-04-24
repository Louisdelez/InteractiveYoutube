# Operations

## Local dev

```bash
cp .env.example .env          # fill YOUTUBE_API_KEY + JWT_SECRET
npm run install:all
npm run dev                    # server :4500 + vite :4501
# in another terminal, for the native app:
cd desktop && cargo run
```

`npm run dev` runs three processes in parallel via `concurrently` :

| Label | Command | Role |
|---|---|---|
| `web` | `nodemon server/index.js` | Express + Socket.IO — pure HTTP/WS (no cron) |
| `worker` | `nodemon --config nodemon.worker.json workers/index.js` | BullMQ + RSS poll + yt-dlp updater + URL pre-resolver |
| `client` | `vite` | Web client dev server on :4501 |

On first boot the **worker** :
1. Downloads `./bin/yt-dlp` from the yt-dlp GitHub release.
2. Builds every channel's playlist from the YouTube API (cached to `server/data/playlist-*.json`). `loadFromDisk` validates `tvStartedAt` / `totalDuration` / non-empty `videos` ; malformed JSON triggers a rebuild instead of serving `NaN`.
3. Runs the initial URL-resolver sweep (~110 s for 52 channels @ concurrency 2) so cold zap serves pre-resolved URLs from Redis.

On first boot the **web** just creates the `users` table in Postgres if missing.

The 48-channel initial playlist build takes 5–15 minutes. Subsequent boots are instant (disk cache). Nodemon is configured with `ignore: ["data/**"]` on both so playlist JSON writes from the worker don't restart the web (the old "nodemon clobbering nightly cron" class of bugs).

## Production — Docker Compose

```bash
docker-compose up -d --build
docker-compose logs -f app
```

Services defined in `docker-compose.yml`:

| Service    | Image             | Ports                    | Volume                    |
| ---------- | ----------------- | ------------------------ | ------------------------- |
| `db`       | `postgres:16-alpine` | —                     | `postgres_data:/var/lib/postgresql/data` |
| `redis`    | `redis:7-alpine`  | —                        | `redis_data:/data`        |
| `app`      | built from `Dockerfile` | internal 4500      | `./logs:/app/logs`, `./server/data:/app/server/data`, `./bin:/app/bin` |
| `nginx`    | `nginx:alpine`    | 80, 443                  | `./nginx/nginx.conf`, `/etc/letsencrypt` |

All services have 5 s health checks; `app` waits for `db` and `redis` to be healthy. Secrets come from the host `.env`.

## Dockerfile

Multi-stage:
1. `node:20-alpine` builder — installs server + client deps, runs `cd client && npm run build`.
2. Runtime stage — `pm2-runtime ecosystem.config.js --env production` under `tini` for clean signal propagation. Exposes 4500.

`tini` is important: without it, PM2's graceful shutdown + Node's `SIGTERM` handlers don't see the container stop signal cleanly.

## PM2 (`ecosystem.config.js`)

Two apps defined :

| App | Role | Cluster |
|---|---|---|
| `koala-tv` | web — Express + Socket.IO | `instances: 'max'` |
| `koala-tv-maint` | maintenance worker — BullMQ + RSS + URL resolver + yt-dlp updater | `instances: 1` |

Worker is always single-instance — the cron schedule lives in Redis via BullMQ so multiple workers would stampede the same job. Web is cluster-mode for HTTP/WS throughput ; Socket.IO uses the Redis adapter so rooms + events fan out across workers.

- `max_memory_restart: 300M`, `node_args: --max-old-space-size=512` on web. Worker headroom is larger (refreshPlaylist holds ~48 playlists in RAM).
- Graceful shutdown : `kill_timeout: 10000`, `listen_timeout: 10000`, `shutdown_with_message: true`.
- `autorestart: true`, `max_restarts: 10`, `restart_delay: 1000`. The daily 3 am maintenance no longer restarts any process — BullMQ retries in-process with checkpoint recovery.
- Logs merged to `./logs/out.log` and `./logs/error.log` with ISO timestamps.

## nginx (`nginx/nginx.conf`)

- **Upstream**: `app:4500` (single service behind the Docker DNS). `ip_hash` so a polling client sticks to the same worker during its session.
- **Rate limits**:
  - `/api/*`: 30 req/s per IP, burst 50
  - `/socket.io/*`: 5 req/s per IP, burst 10
  - `/health`: no limit
- **WebSocket pass-through**: `Upgrade`/`Connection` headers + 86400 s timeouts + `proxy_buffering off` for `/socket.io/`.
- **Static**: `/assets/*` (hashed Vite outputs) cached 1 year immutable. `/` fallback to `index.html` (SPA).
- **Security headers**: X-Frame-Options, X-Content-Type-Options, X-XSS-Protection, Referrer-Policy.
- **SSL**: certificate paths stubbed (commented). Uncomment + mount your Let's Encrypt volume.

## Environment variables

| Variable               | Required | Notes                                                                   |
| ---------------------- | -------- | ----------------------------------------------------------------------- |
| `YOUTUBE_API_KEY`      | ✓        | YouTube Data API v3, project needs the API enabled.                     |
| `JWT_SECRET`           | ✓        | `openssl rand -hex 32`                                                  |
| `DATABASE_URL`         | ✓        | `postgresql://user:pass@host:5432/db` — no default; must be explicit.   |
| `TENOR_API_KEY`        | ✓        | Google Cloud API key with the Tenor API enabled. Used by `/api/gifs/*`. |
| `REDIS_URL`            |          | `redis://host:6379`                                                     |
| `PORT`                 |          | 4500                                                                    |
| `NODE_ENV`             |          | `production` in compose                                                 |
| `CLIENT_ORIGIN`        |          | Public origin for CORS + cookie `SameSite`                              |
| `SERVER_TZ`            |          | `Europe/Paris` by default — controls the 3 am maintenance cron.         |
| `HEALTHCHECKS_URL`     |          | Optional dead-man-switch ping from the maintenance worker.              |
| `DESKTOP_DOWNLOAD_URL` |          | Served by `/api/tv/desktop-download`, shown in the web fallback        |

All four required vars are enforced in `server/config.js` — the process refuses to boot if any are missing. No silent fallbacks for secrets or credentials: previous commits shipped a leaked Tenor API key and default Postgres credentials as fallbacks; both have been removed. Rotate any key that lived in git history.

Never commit the real `.env` — the template is `.env.example`, and `.env` is gitignored.

## Maintenance cron

Lives in the `koala-tv-maint` process. BullMQ scheduler persisted in Redis ; survives worker restarts. Default times come from env (see [SERVER.md](SERVER.md#configuration)).

| Scheduler | Default pattern | Job |
|---|---|---|
| `koala-daily-3am` | `DAILY_REFRESH_CRON` = `0 3 * * *` | 5-step pipeline with Redis checkpoints (TTL 6 h) so a crash mid-run resumes from the last completed step. Steps : `yt-dlp -U` → refresh 1/7 channels → Redis rate-limit key cleanup → chat history wipe + `chat:cleared` broadcast → verify empty. |
| `koala-daily-2h55-warning` | `DAILY_WARNING_CRON` = `55 2 * * *` | Broadcast `maintenance:warning` 5 min before the refresh so clients show the banner. |

**No process restart.** Metrics counters don't reset. Logged-in users stay authenticated. Active HTTP requests keep running (web tier is never touched).

Manual triggers (loopback-only) :

```
POST /api/admin/maintenance-trigger   # enqueues an immediate daily-maintenance job
POST /api/admin/maintenance-reset     # force-resolves a stuck banner
GET  /api/admin/cron-status           # schedulers, waiting/active counts, recent runs
```

Dead-man switch : set `HEALTHCHECKS_URL` to a Healthchecks.io (or compatible) URL ; the worker pings it on success and `/fail` on failure after each daily run.

## URL pre-resolution

The worker pre-resolves googlevideo URLs via yt-dlp for every channel's current video and caches them in Redis (`koala:url:<channelId>`, TTL 1 h). Every `tv:state` / `tv:sync` is enriched with `resolvedUrl` + `resolvedUrlLq` + `resolvedAt`. Desktop clients pass these straight to mpv with `ytdl=no`, skipping the ~200-800 ms yt-dlp step on cold zap.

Event-driven invalidation : on detected video change (auto-advance or priority injection), the cache for that channel is dropped and a fresh resolve is scheduled via `setImmediate`. Env : `URL_RESOLVER_INTERVAL_MS` (30 min sweep), `URL_RESOLVER_CONCURRENCY` (2), `URL_RESOLVER_CACHE_TTL_SECS` (3600), plus the format selectors `URL_RESOLVER_FMT_{MAIN,LQ}`.

## yt-dlp auto-update

Two independent updaters :

- **Server worker** : `server/services/ytdlp-updater.js` manages `./bin/yt-dlp`. Downloads on first run, `-U` every `YTDLP_UPDATE_INTERVAL_MS` (6 h). `server/scripts/ytdlp-audit.sh` and the URL resolver both use this binary. The web tier doesn't touch it.
- **Desktop** : `desktop/src/services/ytdlp_updater.rs` manages `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp`. Same logic. mpv is pointed at this binary via `--script-opts=ytdl_hook-ytdl_path=…` at subprocess spawn time.

Both are intentionally separate so the desktop works offline of the server.

## Load test (`loadtest/artillery.yml`)

```bash
npx artillery run loadtest/artillery.yml
```

Profile:
- Ramp 0 → 100 users / 30 s
- Sustain 100 users / 60 s
- Spike 500 users / 15 s

Two scenarios:
- **70% viewer**: connect, idle 30–60 s, disconnect.
- **30% chatter**: connect, send 5 messages (3 s apart), disconnect.

Target: `http://localhost:4500`. Adjust for your staging host.

## Monitoring

- `GET /health` — lightweight JSON (`{status, uptime}`) for load balancers.
- `GET /metrics` — Prometheus text format. Scrape interval 15–30 s is fine.
- Logs — pino JSON on stdout (redirected to `logs/*.log` by PM2). Ship with Loki / CloudWatch / whatever.

Key metrics to alert on:
- `iy_socket_connections_total` sudden drop → relay issue.
- `iy_tv_sync_broadcast_duration_ms` p99 > 100 ms → Redis or worker overload.
- `iy_auth_attempts_total{status="error"}` spike → credential stuffing.

## Gotchas

- **Apple Silicon** : the desktop spawns `mpv` as a subprocess — only the mpv runtime is needed (`brew install mpv`), no libmpv headers. The X11 overlays (popup menus, tooltip, badge, backup player) are still Linux-only ; on macOS the app today is a "separate window" mpv without in-window embed. See [CROSS_PLATFORM.md](CROSS_PLATFORM.md).
- **Clustered Socket.IO** : always behind nginx `ip_hash` (polling fallback would break otherwise). The Redis adapter handles broadcast fan-out between workers.
- **CORS + cookies** : if the web client is served from a different origin than the API, set `CLIENT_ORIGIN` correctly and put them behind the same parent domain. Cookies are `SameSite=strict` in production.
- **Daily maintenance** : no longer restarts any process. Requests in flight at 03:00 are unaffected. Only side effect is the chat history wipe + client banner.
- **Loopback rate limit exempt** : the Socket.IO IP-rate-limiter (`SOCKET_MAX_CONNECTIONS_PER_IP`) skips `127.0.0.1` / `::1` / `::ffff:127.0.0.1`. Dev `pkill -9` cycles on the desktop used to wedge the local socket once the counter climbed past the cap (no clean disconnect packet → no decrement). Still active on non-loopback traffic.

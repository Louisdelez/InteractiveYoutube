# Operations

## Local dev

```bash
cp .env.example .env          # fill YOUTUBE_API_KEY + JWT_SECRET
npm run install:all
npm run dev                    # server :4500 + vite :4501
# in another terminal, for the native app:
cd desktop && cargo run
```

On first boot the server:
1. Downloads `./bin/yt-dlp` from the yt-dlp GitHub release.
2. Creates the `users` table in Postgres if missing.
3. Builds every channel's playlist from the YouTube API (cached to `server/data/playlist-*.json`).

The 48-channel initial build takes 5–15 minutes. Subsequent boots are instant (disk cache). Background processes (RSS poll, daily cron, yt-dlp updater) run even in dev.

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

- `instances: 'max'` → one worker per CPU core (cluster mode).
- `max_memory_restart: 300M`, `node_args: --max-old-space-size=512`.
- Graceful shutdown: `kill_timeout: 10000`, `listen_timeout: 10000`, `shutdown_with_message: true`.
- `autorestart: true`, `max_restarts: 10`, `restart_delay: 1000` — the daily 3 am `process.exit(1)` is well within this budget.
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

`server/cron/refresh.js` runs inside the Node process (no system crontab). Every day at **03:00 local server time**:

1. Refresh 1/7th of the channels via the YouTube API (bucket = `index % 7 === dayOfWeek`). Uses timecode-preserving merge.
2. Wipe all Redis chat histories (`SCAN chat:history:* | DEL`) and broadcast `chat:cleared` so clients clear their UI.
3. `process.exit(1)` → PM2 respawns the workers. The 2 s delay gives logs and sockets time to flush.

Side effects you should be aware of:
- Metrics counters reset (PM2 respawn creates fresh prom-client registries).
- Logged-in users stay authenticated (JWT cookie survives).
- Active HTTP requests are aborted — clients retry.

## yt-dlp auto-update

Two independent updaters:

- **Server**: `server/services/ytdlp-updater.js` manages `./bin/yt-dlp`. Downloads on first run, `-U` every 6 h. `server/scripts/ytdlp-audit.sh` and the server itself use this binary.
- **Desktop**: `desktop/src/services/ytdlp_updater.rs` manages `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp`. Same logic. libmpv is pointed at this binary via `script-opts=ytdl_hook-ytdl_path=…`.

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

- **Apple Silicon**: libmpv for the desktop app builds fine via Homebrew, but the dev experience is Linux-first; we've never shipped a macOS build.
- **Clustered Socket.IO**: always behind nginx `ip_hash` (polling fallback would break otherwise). The Redis adapter handles broadcast fan-out between workers.
- **CORS + cookies**: if the web client is served from a different origin than the API, set `CLIENT_ORIGIN` correctly and put them behind the same parent domain. Cookies are `SameSite=strict` in production.
- **Daily restart and long-running HTTP**: don't park an upload or long-poll at 02:59 — it'll be cut at 03:00:02.

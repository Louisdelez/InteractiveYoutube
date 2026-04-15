<p align="center">
  <img src="client/public/koala-tv.png" alt="Koala TV" width="160" />
</p>

<h1 align="center">Koala TV</h1>

<p align="center">
  A synced multi-channel YouTube TV — same video, same second, for every viewer on a channel.<br />
  Web client + native desktop app + Node.js server, powered by YouTube playlists and a shared live chat.
</p>

<p align="center">
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-8b5cf6" /></a>
  <img alt="Node 20+" src="https://img.shields.io/badge/node-20+-3c873a" />
  <img alt="Rust stable" src="https://img.shields.io/badge/rust-stable-ce422b" />
  <img alt="React 19" src="https://img.shields.io/badge/react-19-61dafb" />
</p>

---

Koala TV schedules YouTube videos on fixed "channels" and broadcasts the exact playhead position to every connected viewer. Drop in, you land right where the others are — no rewind, no catch-up.

- **Server** (Node.js + Express + Socket.IO) — the TV scheduler and source of truth. Drives ~48 channels built from YouTube creator uploads, holds the wallclock, broadcasts drift corrections.
- **Web client** (Vite + React) — browser app that embeds a YouTube iframe and falls back to a YouTube deep-link with live timecode when a video is not embeddable.
- **Desktop app** (Rust + GPUI + libmpv) — native player that streams any YouTube video directly (no iframe restriction) using an auto-updated `yt-dlp` and a **dual-mpv "zero cut"** playback pipeline.

## Features

- **Real-time sync** across all connected clients (15 s server tick + >4 s drift correction on each client).
- **48 French creator channels** out of the box, each built from one or more YouTube channels / playlists.
- **Append-only, timecode-preserving refresh**: new uploads are spliced in without rewinding the TV; the cycle-relative position is exact across refreshes and restarts.
- **Daily 3 am maintenance**: 1/7th of the channels get a full YouTube-API refresh, all chat histories are wiped, the process is restarted — spread API quota, keep things fresh.
- **yt-dlp auto-update** on both server and desktop (bootstraps the binary, then `-U` every 6 h).
- **Live chat** with Redis-backed history, rate-limiting, anonymous pseudos (animals/fruits/vegetables), and optional user accounts.
- **Embed fallback on web**: when YouTube refuses embedding, the player area shows a live-timecoded "Watch on YouTube" link and an "Install the desktop app" button — chat and channel switching stay fully usable.
- **Dual-mpv zero-cut playback on desktop**: a low-quality backup stream is always ready to take over if the main stream stalls, then swaps back up once the main has a decoded frame. No black screens, no audio gaps.

## Quick start (dev)

### Prerequisites

- Node.js 20+, PostgreSQL 16, Redis 7
- Rust (stable) + libmpv dev headers for the desktop app (`libmpv-dev` on Debian/Ubuntu)
- A Google Cloud project with the YouTube Data API v3 enabled

### Server + web

```bash
cp .env.example .env   # fill in YOUTUBE_API_KEY and JWT_SECRET
npm run install:all    # root + server + client npm installs
# in one terminal
npm run dev            # server on :4500 + vite on :4501
```

The first boot of the server downloads `./bin/yt-dlp` and then builds every channel's playlist from the YouTube API (a few minutes the first time, cached to `server/data/playlist-*.json` afterwards).

Open http://localhost:4501.

### Desktop

```bash
cd desktop
cargo run              # pulls libmpv, builds, launches
```

On first launch the desktop app downloads its own `yt-dlp` to `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp`.

## Production

A `docker-compose.yml` ships the full stack (PostgreSQL, Redis, Node cluster under PM2, nginx reverse proxy). See [`docs/OPERATIONS.md`](docs/OPERATIONS.md).

## Repository layout

```
.
├── server/           # Node/Express/Socket.IO backend + cron + yt-dlp updater
│   ├── cron/         # Daily 3 am refresh + chat wipe + restart
│   ├── routes/       # /api/auth, /api/tv, /api/user
│   ├── services/     # playlist builder, tv state, redis, metrics, ytdlp-updater
│   ├── socket/       # Socket.IO handlers (tv, chat, rooms)
│   └── config.js     # 48 channels, env, cron schedule
├── client/           # Vite + React web app (+ optional Tauri shell)
│   └── src/
│       ├── components/   # Player, PlayerFallback, ChannelSidebar, Chat, …
│       ├── hooks/        # useTvSync, useChat, useAuth, useSocket
│       └── services/     # api, socket, platform, pseudoGenerator
├── desktop/          # Rust + GPUI + libmpv native TV app
│   └── src/
│       ├── views/    # player (dual-mpv), sidebar, chat, popup_menu, tooltip…
│       ├── services/ # api, websocket (socket.io), ytdlp_updater, settings
│       └── models/   # channel, message, tv_state
├── nginx/            # Reverse proxy config (rate limit + WebSocket pass-through)
├── loadtest/         # Artillery Socket.IO load scenarios
├── docker-compose.yml
├── Dockerfile
├── ecosystem.config.js  # PM2 cluster mode
└── docs/
    ├── ARCHITECTURE.md    # System diagram, data flow
    ├── SERVER.md          # HTTP + Socket API, services, cron
    ├── CLIENT.md          # React components, hooks, fallback flow
    ├── DESKTOP.md         # GPUI/libmpv, dual-mpv, X11
    └── OPERATIONS.md      # Docker, PM2, nginx, env, load tests
```

## How it works (one paragraph)

Each channel has a shuffled playlist and a `tvStartedAt` timestamp. The currently-playing video and its in-video seek position are pure functions of wallclock time: `elapsed = (now - tvStartedAt) mod totalDuration`, then a binary search on prefix sums gives the index + offset. The server broadcasts this full state on connect and every 15 seconds (`tv:sync`); clients keep their own player synced and resync if the drift exceeds 4 seconds. New uploads are appended **without changing** the existing order or durations, and `tvStartedAt` is rebased so the cycle-relative position stays exact. On restart, the state is rehydrated from disk (`server/data/playlist-*.json`), and virtual time keeps flowing during the downtime.

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — high-level diagram and data flow
- [Server](docs/SERVER.md) — HTTP API, Socket.IO events, services, cron
- [Web client](docs/CLIENT.md) — components, hooks, sync logic, fallback
- [Desktop app](docs/DESKTOP.md) — GPUI + libmpv, dual-mpv pattern, X11
- [Operations](docs/OPERATIONS.md) — Docker, PM2, nginx, env, load tests

## License

[MIT](LICENSE) © Louis Delez

# Koala TV — context for Claude Code

> This file is auto-loaded by Claude Code when the agent runs in this directory. It consolidates the cross-session memory so the repo is portable (Linux → Windows / Mac, any machine). Update it when state changes.

## User — Louis Delez (francophone)

- GitHub: `Louisdelez` · email: `loicdelez.ch@gmail.com`
- Prefers **maximum autonomy**: agir directement, pas de confirmation inutile. Quand l'action est risquée (destruction, push public), demander une seule fois puis exécuter.
- Dev **depuis Linux uniquement** — pas de Mac ni de Windows physique sur place.

## Project — Koala TV (ex-InteractiveYoutube)

Multi-channel synced YouTube TV (web + native desktop + server).

- **GitHub**: https://github.com/Louisdelez/KoalaTV (renommé de `InteractiveYoutube` — l'ancien URL redirige)
- **License**: MIT, © Louis Delez
- **Branch**: `main`. Git identity locale au repo: `loicdelez.ch@gmail.com` / "Louis" (config locale, pas `--global`)
- **Repo path (Linux)**: `/home/louis/Documents/InteractiveYoutube` (dossier non renommé pour compat; le slug affiché est "Koala TV")
- **Local repo path on Windows**: variable selon la machine — adapte les chemins absolus.

### Stack

- **Web client** (`client/`): Vite + React 19 (port 4501). Proxy dev `/api` + `/socket.io` → :4500. npm slug `koala-tv`. Tauri shell optionnel.
- **Desktop** (`desktop/`): Rust + GPUI (Zed, git) + gpui-component (Longbridge) + libmpv2 5 + rust_socketio + x11-dl (Linux only). Cargo name `koala-tv-desktop`.
- **Server** (`server/`): Node 20 + Express + Socket.IO 4.8 + Redis adapter + PostgreSQL. Port 4500. PM2 app name `koala-tv`.
- **Ops**: `docker-compose.yml`, `ecosystem.config.js`, `nginx/`, `loadtest/`, `.github/workflows/desktop-ci.yml` (matrix ubuntu/macos-13/macos-14/windows).

### Source of truth architecture

- Le serveur tient **toute** l'autorité : playlist, seed de shuffle, `tvStartedAt`, priority queue, chat.
- Clients (web + desktop) : pure projection. `elapsed = (now - tvStartedAt) mod totalDuration` + binary search sur prefix sums = video courante + seekTo.
- Clock offset = médiane de 5 ping-pong RTT halvings, recomputé à chaque reconnect.
- Drift correction : si `|player.currentTime - expected| > 4 s`, hard seek.

### Automations critiques

- **Daily 3 am cron** (`server/cron/refresh.js`, `DAILY_REFRESH_CRON='0 3 * * *'`):
  1. Refresh 1/7th des chaînes (bucket = `index % 7 === new Date().getDay()`) via YouTube API — spread quota.
  2. `clearAllChatHistory(io)` : SCAN `chat:history:*` + DEL + broadcast `chat:cleared`.
  3. `setTimeout(() => process.exit(1), 2000)` → PM2/nodemon respawn.
- **RSS poll toutes les 30 min** pour nouvelles uploads → `queuePriorityVideo` + `addNewVideos` + broadcast `tv:newRelease`.
- **yt-dlp auto-update toutes les 6 h** (serveur ET desktop indépendamment):
  - Serveur: `<repo>/bin/yt-dlp` (gitignored). Bootstrap GitHub releases puis `-U`.
  - Desktop: `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp` (ou `~/.local/share/KoalaTV/bin/` ; sur Windows: `%APPDATA%\KoalaTV\bin\` — non encore wiré).
  - Desktop injecte le path dans mpv via `script-opts=ytdl_hook-ytdl_path=<path>`.

### Timecode-preserving refresh (règle d'or)

`mergePlaylistPreservingTimecode(oldState, newVideos)` dans `server/services/playlist.js`:

1. Append only les videos vraiment nouvelles (diff vs IDs connus).
2. Rebase `tvStartedAt = now - elapsedInCycle` où `elapsedInCycle = (now - oldStart) mod oldTotal`.
3. → garantit `(now - newStart) mod newTotal === elapsedInCycle` même si `totalDuration` change et même après X cycles.

Appelé par `refreshPlaylist()` (daily cron) ET `addNewVideos()` (RSS poll). Résultat: viewer voit le même frame avant et après le merge.

### User-facing paths / IDs

- Desktop settings: `~/.config/koala-tv/settings.json` (Linux) / `%APPDATA%\koala-tv\settings.json` (Windows, pas encore wiré)
- Desktop yt-dlp binary: `$XDG_DATA_HOME/KoalaTV/bin/yt-dlp`
- Window title: "Koala TV"
- Logo embedded au compile time: `desktop/assets/koala-tv.png` via `include_bytes!` → `OnceLock<Arc<Image>>` dans `app.rs`
- Web favicon: `client/public/koala-tv.png`
- Tauri productName / identifier: "Koala TV" / `com.koalatv.app`

### Serveur — endpoints principaux

- `GET /health`, `GET /metrics` (Prometheus)
- `POST /api/auth/{login,register,logout}` — JWT cookie HttpOnly SameSite=strict, rate-limited
- `GET /api/auth/me` — session probe
- `GET/PUT /api/user/settings` — settings synchro serveur (logged in only)
- `GET /api/tv/state?channel=X` — état TV (rarement utilisé — Socket.IO fait le gros)
- `GET /api/tv/channels` — `[{id, name, handle, avatar}]` — source of truth pour les sidebars web + desktop
- `GET /api/tv/desktop-download` — URL fallback web "télécharger l'app" (configurable via `DESKTOP_DOWNLOAD_URL`)

### Socket.IO events

Server → client: `tv:state`, `tv:sync` (15 s tick), `tv:refreshed`, `tv:newRelease`, `tv:pong`, `chat:history`, `chat:batch`, `chat:cleared`, `chat:error`, `viewers:count`.

Client → server: `tv:ping`, `tv:switchChannel` (debounced 400 ms), `tv:requestState`, `tv:videoError`, `chat:message`, `chat:channelChanged`, `chat:setAnonymousName`.

## Cross-platform status

**Phase 1 DONE** (code compile sur 4 targets, Linux 100% fonctionnel):

- X11-only modules gated `#![cfg(target_os = "linux")]`: `backup_player`, `popup_menu`, `xft_text`, `memory_cache`, `channel_badge`, `loading_overlay`.
- `tooltip.rs` a un stub no-op non-Linux.
- `player.rs` : ~44 cfg gates internes ; non-Linux lance un mpv "fenêtre séparée" sans embed.
- `app.rs` : 10+ gates.
- CI matrix `.github/workflows/desktop-ci.yml` (ubuntu-22.04 / macos-13 / macos-14 / windows-latest).
- Vérifié: `cargo check --target x86_64-pc-windows-gnu` passe (warnings only).

**Phase 2 TODO** (in-window mpv embed via `raw-window-handle`: `wid=HWND` Windows, `wid=NSView*` macOS).

**Phase 3 TODO** (remplacer X11 overlays par GPUI natif cross-platform — accepter de perdre le dual-mpv zero-cut hors Linux en v1).

**Phase 4 TODO** (packaging `cargo-bundle`/`cargo-wix`, signing optionnel, release CI sur tag).

Plan complet: `docs/CROSS_PLATFORM.md`.

### Contraintes de test

- **Windows**: réalisable 100% depuis Linux via VM QEMU/KVM + image gratuite Microsoft « Windows 11 Dev Environment » (licence 90 j renouvelable, 8 GB RAM / 60 GB disque). Alternative plus légère: cross-compile via `cross`+mingw-w64 + Wine pour smoke test.
- **macOS**: **virtualiser sur hardware non-Apple viole l'EULA** (section 2B). Pas acceptable pour un projet open-source. Chemins légaux:
  1. Cloud Mac à l'heure (Scaleway M1 ~1 €/h, MacStadium ~50-80 €/mois, MacinCloud ~30 $/mois)
  2. GitHub Actions macos-14 runner (gratuit 2h/job) + `tmate` action pour SSH/VNC hacky
  3. Community beta testing
- **Signing**: macOS Apple Dev 99 $/an (ou ship non-signé avec warning Gatekeeper); Windows code-signing ~200 $/an (ou accepter SmartScreen). OK de ship non-signé en v1.

## Docs dans le repo

- `README.md` — overview + quick start
- `docs/ARCHITECTURE.md` — diagramme système, timecode math
- `docs/SERVER.md` — API HTTP + Socket.IO + services
- `docs/CLIENT.md` — React components + hooks + fallback iframe flow
- `docs/DESKTOP.md` — GPUI + libmpv + dual-mpv + X11
- `docs/OPERATIONS.md` — Docker + PM2 + nginx + env vars
- `docs/CROSS_PLATFORM.md` — plan Linux / Windows / macOS

## Pièges techniques importants

- **X11 ARGB visual**: `XBlackPixel` retourne `0x00000000` sur visual 32-bit = **transparent** (bureau visible à travers). Toujours utiliser `0xFF000000` explicite + `XSetWindowBackground` + `XClearWindow` sur toutes les fenêtres X11 child.
- **mpv `force-window=yes` se re-mappe**: un `XUnmapWindow` sur la fenêtre mpv ne tient pas. Utiliser `XMoveWindow(-10000,-10000)` pour cacher sans re-map.
- **rust_socketio race au connect**: `socket.emit()` juste après `connect()` Ok retourne `true` mais le packet est silently droppé. Délai ~150 ms après connect.
- **rust_socketio `open` event pas fiable**: émettre `Connected` sur succès de `connect()` direct, pas via le `.on("open")`.
- **mpv `VIDEO_RECONFIG` vs `PlaybackRestart`**: `VIDEO_RECONFIG` = première frame décodée prête VO (le bon signal pour "vidéo à l'écran"). `PlaybackRestart` fire trop tôt.
- **mpv loadfile: drain events AVANT** pas après — sinon le `VIDEO_RECONFIG` du nouveau fichier est consommé et perdu.
- **`time-pos` saute à `start` instantanément** sur loadfile — inutilisable comme signal de "playback en cours".
- **Drift-aware seek** sur swap dual-mpv: skip si <300 ms (up) ou <500 ms (down), sinon hr-seek → re-buffer noir.
- **`b.move_offscreen()` vs `b.hide()` (XUnmap)** sur swap-up: `XUnmap` = compositor expose = 1-frame black flash. Off-screen = pas de flash.
- **nodemon clean exit ≠ restart**: `process.exit(0)` sur nodemon = `[nodemon] clean exit - waiting for changes`. Utiliser `process.exit(1)` pour que PM2 **ET** nodemon respawnent.

## Règles de collaboration apprises

- **Autonomie par défaut** : ne pas redemander confirmation pour les opérations locales réversibles (edit, lancer l'app en dev, commit). Demander pour: push force, destruction de données, opérations coûteuses long-running.
- **Réponses courtes** : Louis lit le diff, pas besoin de re-narrer ce qui a été fait, juste le résultat et le next step éventuel.
- **Français par défaut** dans les échanges ; commits + code commentaires en anglais.
- **Preview image nouveau champ dans repo** : si une image est fournie, utiliser `/koala-tv.png` comme brand partout (favicon web + logo topbar + desktop embed via `include_bytes!`).

# Architecture

## System diagram

```
                       ┌──────────────────────────┐
                       │  YouTube Data API v3     │
                       │  + yt-dlp (stream URLs,  │
                       │  pre-resolved server-side│
                       │  then cached in Redis)   │
                       └────────────┬─────────────┘
                                    │
                                    ▼
 ┌────────┐   Socket.IO    ┌──────────────────────┐    HTTP + Socket.IO
 │ Web    │◀──────────────▶│   Web process        │◀────────────────────┐
 │ client │                │   koala-tv           │                     │
 │ (Vite  │                │   (PM2 cluster)      │                     │
 │ +React)│   HTTP /api/*  │ - HTTP routes        │                     │
 └────────┘◀──────────────▶│ - Socket.IO + Redis  │                     │
                           │   adapter            │                     │
                           │ - Sync broadcaster   │                     │
                           │ - Chat relay         │                     │
                           └──────┬───────┬───────┘                     │
                                  │       │                             │
                          ┌───────▼──┐  ┌─▼────────┐           ┌────────┴──────┐
                          │ Redis    │  │ Postgres │           │ Desktop app   │
                          │ (chat,   │  │ (users,  │           │ (Rust + GPUI  │
                          │  BullMQ, │  │  settings│           │  + 2× mpv     │
                          │  URL     │  │  JSONB)  │           │  subprocesses │
                          │  cache,  │  │          │           │  via IPC,     │
                          │  viewers)│  │          │           │  own yt-dlp)  │
                          └─────▲────┘  └──────────┘           └───────────────┘
                                │
                                │ BullMQ + pub/sub
                                │
                           ┌────┴─────────────────┐
                           │  Worker process      │
                           │  koala-tv-maint      │
                           │  (single instance)   │
                           │ - BullMQ scheduler   │
                           │ - Daily maintenance  │
                           │ - RSS poll (30 min)  │
                           │ - yt-dlp updater     │
                           │ - URL pre-resolver   │
                           └──────────────────────┘
```

Two Node processes, three roles :
- **`koala-tv`** (web, cluster) — pure HTTP/WS, no scheduled work.
- **`koala-tv-maint`** (worker, single) — all scheduled + background tasks.
- **Redis** — message bus between the two (BullMQ for jobs, pub/sub for state changes, `@socket.io/redis-emitter` for client fan-out from the worker).

## Source of truth

The **server** holds all state that must be shared:
- Per-channel shuffled playlist (built from YouTube, cached on disk)
- `tvStartedAt` — the wallclock epoch of "index 0, offset 0"
- Priority queue for fresh uploads that must play next
- Chat history in Redis
- Viewer counts in Redis

The **web** and **desktop** clients are pure projections. They:
1. Compute a clock offset against the server on connect (5-ping median).
2. Receive a full `tv:state` on connect / channel switch.
3. Receive `tv:sync` every 15 s and resync if `|drift| > 4 s`.
4. Send chat messages and channel-switch commands.

## Data flow: "what is playing right now?"

For a given channel:

```
elapsed  = (now - tvStartedAt) mod totalDuration
index    = binary_search(prefixSums, elapsed)     // O(log N)
seekTo   = elapsed - prefixSums[index - 1]
```

`prefixSums` is a `Float64Array` of cumulative video durations, built once on playlist load. This gives sub-microsecond lookups even for 6 000-video playlists.

If there's a priority video pending (new upload detected by the 30-min RSS poll), it takes over the next rotation boundary; once it ends, the pointer snaps back to where the normal rotation would have been at that moment.

## Timecode-preserving refresh

The hard part: YouTube keeps publishing. How do you add a new video to a running TV channel without the viewer seeing a skip?

**Append-only merge** (`server/services/playlist.js`):

1. Keep every old video in its exact slot (same index, same duration, same metadata) → `prefixSums` up to `oldLen` is bit-identical.
2. Append truly-new videos at the end.
3. Rebase `tvStartedAt` so the cycle-relative position is preserved across the `totalDuration` growth:

```js
elapsedInCycle = (now - oldStart) mod oldTotal
newStart       = now - elapsedInCycle
// → (now - newStart) mod newTotal === elapsedInCycle
```

Result: the currently-playing video is still at exactly the same second before and after the merge. The new videos first play when the TV naturally reaches the end of its previous total duration.

## Daily 3 am maintenance

BullMQ scheduler persisted in Redis (`server/workers/daily-maintenance.js`). The worker process owns the schedule ; the web tier is never restarted.

1. `yt-dlp -U` (self-update).
2. **Refresh 1/7th of the channels** (`bucket = index % 7 === dayOfWeek`) — YouTube Data API refresh, timecode-preserving merge, 60 s timeout per channel, failures skip.
3. Redis rate-limit key cleanup.
4. **Wipe chat history** (`SCAN chat:history:* | DEL`) and broadcast `chat:cleared` via the socket.io Redis adapter.
5. Verify empty (re-SCAN + force DEL of any reliquats).

Per-step checkpoint in Redis key `maint:ckpt:<jobId>` (TTL 6 h). A crash or retry mid-pipeline resumes from the last completed step instead of restarting from scratch. Retry policy : `attempts: 3`, exponential backoff 60 s. No process restart, no downtime — active requests and Socket.IO sessions are unaffected.

A second scheduler (`koala-daily-2h55-warning`, pattern `55 2 * * *`) broadcasts `maintenance:warning` 5 min before so clients render the banner.

## URL pre-resolution (cold-zap optimisation)

`server/workers/url-resolver.js` runs yt-dlp `-g` for every channel's current video every 30 min and caches the resulting HLS manifest URLs in Redis (`koala:url:<channelId>`, TTL 1 h). Both `tv:state` (HTTP + Socket.IO) and `tv:sync` (every 15 s) are enriched with `resolvedUrl` + `resolvedUrlLq` + `resolvedAt`.

Desktop clients pass the pre-resolved URL to mpv with `ytdl=no`, skipping the ~200-800 ms yt-dlp subprocess spawn client-side. Cold-zap first-frame drops from ~300 ms to ~100 ms.

Event-driven invalidation : `getTvState()` detects videoId changes against the `lastVideoIds` map (auto-advance or priority injection), drops the Redis entry, and schedules a fresh resolve via `setImmediate`. Together with the 15 s `tv:sync` enrichment, cache-hit rate sits near 100 % in steady state.

## Client-side sync loop

Every client (web + desktop) runs the same conceptual loop:

```
on connect:
  offset = median(5 * ping-pong-rtt-half)
  state  = fetch /api/tv/state?channel=…
  play(state.videoId, state.seekTo)

every tv:sync:
  expected = state.seekTo + (now - (serverTime - offset)) / 1000
  if |player.currentTime() - expected| > 4s:
    player.seekTo(expected)

on tv:state:
  // new video (EOF or priority or channel switch)
  load(state.videoId, state.seekTo)
```

The 4 s tolerance is the difference between perceptible and imperceptible drift. Below it, jitter and pause/resume are absorbed; above it, we force a hard seek.

## Embed fallback (web only)

When YouTube refuses to embed a video (creator restriction), the server still knows the correct timecode. The web client replaces the iframe with a panel showing:
- "Watch on YouTube" link with `?t=<seconds>` updated every second
- "Install the desktop app" button (URL served by `/api/tv/desktop-download`)

Sidebar + chat remain fully functional ; the playhead continues advancing server-side, so when the next (embeddable) video starts, the iframe reappears automatically. The desktop app never hits this path — it spawns mpv as a subprocess over JSON IPC and either plays the server-pre-resolved HLS URL (ytdl=no fast path) or falls back to `youtube.com/watch?v=<id>` with mpv's ytdl_hook.

## Horizontal scaling

The Node.js server is stateless. Scaling is done by:
- PM2 in cluster mode (`instances: 'max'` — one worker per CPU core) on a single host.
- Socket.IO with the Redis adapter for cross-worker message fan-out.
- nginx with `ip_hash` so long-polling fallbacks stick to the same worker.

Shared state lives in Redis (chat, rate limits, viewer sets) and PostgreSQL (users, settings). Disk playlists can be NFS-mounted or rebuilt from the YouTube API on boot.

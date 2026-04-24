/**
 * yt-dlp URL pre-resolver.
 *
 * The expensive part of a cold mpv loadfile on the desktop client is
 * the ytdl_hook → yt-dlp subprocess that resolves youtube.com/watch?v=X
 * into a final googlevideo.com streaming URL (python startup + YouTube
 * page fetch + signature/n-param decipher ≈ 200–800 ms per click).
 *
 * We pre-resolve on the server once per channel and cache the result in
 * Redis. Clients then pass the already-resolved URL straight to
 * mpv.loadfile, bypassing yt-dlp entirely (desktop sets ytdl=no for that
 * loadfile). Net effect: cold zap first-frame drops from ~300 ms to
 * ~100 ms (the remaining cost is just TCP + initial HTTP buffer fill).
 *
 * Why single-file progressive formats (not DASH):
 *  - `best[height<=720]`  returns ONE URL (video+audio muxed). mpv plays
 *    it directly, no --audio-file dance, no DASH manifest.
 *  - 720p is the max for YouTube progressive streams; 1080p+ is DASH
 *    only. That's a deliberate trade: instant zap at 720p beats a 1–3 s
 *    delay to get the extra resolution. Users who want 1080p can flip
 *    the Settings → Quality menu, which forces a normal ytdl-hook
 *    loadfile with the full `bestvideo+bestaudio` format selector.
 */
const { spawn } = require('child_process');
const path = require('path');
const config = require('../config');
const log = require('./logger').child({ component: 'url-resolver' });
const { redis } = require('./redis');

const YTDLP_BIN =
  process.env.YTDLP_BIN_PATH ||
  path.resolve(process.env.YTDLP_BIN_DIR || path.resolve(__dirname, '..', '..', 'bin'), 'yt-dlp');

// yt-dlp format selectors. `[vcodec!*=av01]` matches the desktop client's
// existing preference — AV1 hardware decode is inconsistent on GTX 16xx
// and below, so we stick to h264/vp9.
const FMT_MAIN = process.env.URL_RESOLVER_FMT_MAIN || 'best[height<=720][vcodec!*=av01]/best[vcodec!*=av01]/best';
const FMT_LQ = process.env.URL_RESOLVER_FMT_LQ || 'worst[height<=360][vcodec!*=av01]/worst[vcodec!*=av01]/worst';

// How long a cached entry is considered valid server-side. YouTube
// googlevideo tokens typically expire ~6 h after issuance; we re-resolve
// every 30 min and keep the cache TTL at 1 h so we always have a fresh
// entry ready. Set via env for ops tuning.
const CACHE_TTL_SECS = parseInt(process.env.URL_RESOLVER_CACHE_TTL_SECS) || 3600;
const RESOLVE_TIMEOUT_MS =
  parseInt(process.env.URL_RESOLVER_TIMEOUT_MS) || 25_000;

const keyFor = (channelId) => `koala:url:${channelId}`;

/**
 * Run yt-dlp once with the given format selector. Returns the final
 * googlevideo URL (first line of stdout), or throws on any failure
 * (bad exit code, timeout, empty output). Uses `-g` so yt-dlp prints
 * just the URL(s), nothing else.
 */
function ytdlpResolve(videoId, format) {
  return new Promise((resolve, reject) => {
    const args = [
      '-f', format,
      '-g',
      '--no-warnings',
      '--no-playlist',
      '--socket-timeout', '10',
      `https://www.youtube.com/watch?v=${videoId}`,
    ];
    const child = spawn(YTDLP_BIN, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    let err = '';
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill('SIGKILL');
    }, RESOLVE_TIMEOUT_MS);

    child.stdout.on('data', (d) => { out += d.toString(); });
    child.stderr.on('data', (d) => { err += d.toString(); });
    child.on('error', (e) => {
      clearTimeout(timer);
      reject(e);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      if (timedOut) return reject(new Error(`yt-dlp timeout after ${RESOLVE_TIMEOUT_MS}ms`));
      if (code !== 0) return reject(new Error(`yt-dlp exit ${code}: ${err.trim().slice(0, 200)}`));
      const url = out.split('\n').map((s) => s.trim()).find(Boolean);
      if (!url || !url.startsWith('http')) return reject(new Error('yt-dlp: no URL in output'));
      resolve(url);
    });
  });
}

/**
 * Resolve both HQ + LQ URLs for a videoId and cache under channelId.
 * Returns the cached payload. Swallows per-format failures (caches
 * whichever succeeded, logs the failure).
 */
async function resolveAndCache(channelId, videoId) {
  const [mainR, lqR] = await Promise.allSettled([
    ytdlpResolve(videoId, FMT_MAIN),
    ytdlpResolve(videoId, FMT_LQ),
  ]);

  const mainUrl = mainR.status === 'fulfilled' ? mainR.value : null;
  const lqUrl = lqR.status === 'fulfilled' ? lqR.value : null;

  if (!mainUrl && !lqUrl) {
    const e1 = mainR.status === 'rejected' ? mainR.reason.message : '';
    const e2 = lqR.status === 'rejected' ? lqR.reason.message : '';
    throw new Error(`both formats failed: main="${e1}" lq="${e2}"`);
  }

  const payload = {
    videoId,
    mainUrl,
    lqUrl,
    resolvedAt: Math.floor(Date.now() / 1000),
  };

  await redis.set(keyFor(channelId), JSON.stringify(payload), 'EX', CACHE_TTL_SECS);

  if (mainR.status === 'rejected') {
    log.warn({ channelId, videoId, err: mainR.reason.message }, 'url-resolver: main format failed');
  }
  if (lqR.status === 'rejected') {
    log.warn({ channelId, videoId, err: lqR.reason.message }, 'url-resolver: lq format failed');
  }

  return payload;
}

/**
 * Read cached payload for a channel. Returns null if missing, stale
 * (TTL already gone — shouldn't happen given Redis EX, but defensive),
 * or if the cached videoId doesn't match the caller's expectation
 * (caller passes `expectedVideoId` to avoid serving an URL that
 * belongs to the PREVIOUS video on the channel after an auto-advance).
 */
async function getCached(channelId, expectedVideoId) {
  try {
    const raw = await redis.get(keyFor(channelId));
    if (!raw) return null;
    const payload = JSON.parse(raw);
    if (expectedVideoId && payload.videoId !== expectedVideoId) return null;
    return payload;
  } catch (err) {
    log.warn({ channelId, err: err.message }, 'url-resolver: cache read failed');
    return null;
  }
}

async function invalidate(channelId) {
  try { await redis.del(keyFor(channelId)); } catch (_) {}
}

module.exports = {
  resolveAndCache,
  getCached,
  invalidate,
  CACHE_TTL_SECS,
};

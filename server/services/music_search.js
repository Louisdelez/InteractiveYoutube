/**
 * (artist, title) → YouTube video ID resolver for the Hits du Moment
 * channel.
 *
 * Uses `yt-dlp "ytsearch1:<query>"` rather than the YouTube Data
 * API : we already own + auto-update a yt-dlp binary for the daily
 * maintenance worker, and `ytsearch` doesn't burn Data API quota
 * (100 units / search would cap us around 100 lookups/day under the
 * default 10 000 quota).
 *
 * Aggressive Redis cache (90 d TTL) keyed on a normalized
 * `<artist>|<title>` so re-fetching the chart for the same hits has
 * zero yt-dlp cost after day 1. Negative results cached shorter
 * (24 h) so transient YouTube hiccups don't pin a track as
 * "unfindable" for three months.
 */
const { spawn } = require('child_process');
const path = require('path');
const log = require('./logger').child({ component: 'music-search' });
const { redis } = require('./redis');

const YTDLP_BIN =
  process.env.YTDLP_BIN_PATH ||
  path.resolve(process.env.YTDLP_BIN_DIR || path.resolve(__dirname, '..', '..', 'bin'), 'yt-dlp');

const SEARCH_TIMEOUT_MS =
  parseInt(process.env.MUSIC_SEARCH_TIMEOUT_MS) || 20_000;
const CACHE_TTL_HIT_SECS =
  parseInt(process.env.MUSIC_SEARCH_CACHE_HIT_SECS) || 90 * 24 * 3600;
const CACHE_TTL_MISS_SECS =
  parseInt(process.env.MUSIC_SEARCH_CACHE_MISS_SECS) || 24 * 3600;
const NEGATIVE_SENTINEL = '__miss__';

/**
 * Normalize an artist/title pair into a stable cache key. Lower-case,
 * accents stripped, non-alphanumeric collapsed. Keeps `|` as
 * separator. Means "Squeezie - A Fond" and "squeezie a fond" share
 * the same cache entry.
 */
function cacheKey(artist, title) {
  const norm = (s) =>
    String(s)
      .normalize('NFD')
      .replace(/[̀-ͯ]/g, '')
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, ' ')
      .trim();
  return `koala:music-search:${norm(artist)}|${norm(title)}`;
}

function spawnYtdlpSearch(query) {
  return new Promise((resolve, reject) => {
    // `ytsearch1:` = first YouTube search result. `--print id` avoids
    // any JSON parsing — we just want the ID back on stdout, one line.
    const args = [
      `ytsearch1:${query}`,
      '--print', 'id',
      '--no-warnings',
      '--skip-download',
      '--no-playlist',
      '--socket-timeout', '10',
    ];
    const child = spawn(YTDLP_BIN, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    let err = '';
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill('SIGKILL');
    }, SEARCH_TIMEOUT_MS);

    child.stdout.on('data', (d) => { out += d.toString(); });
    child.stderr.on('data', (d) => { err += d.toString(); });
    child.on('error', (e) => { clearTimeout(timer); reject(e); });
    child.on('close', (code) => {
      clearTimeout(timer);
      if (timedOut) return reject(new Error(`yt-dlp timeout after ${SEARCH_TIMEOUT_MS}ms`));
      if (code !== 0) return reject(new Error(`yt-dlp exit ${code}: ${err.trim().slice(0, 200)}`));
      const id = out.split('\n').map((s) => s.trim()).find(Boolean);
      if (!id) return reject(new Error('yt-dlp: empty result'));
      // `ytsearch` returns a plain 11-char YouTube video ID.
      if (!/^[A-Za-z0-9_-]{11}$/.test(id)) {
        return reject(new Error(`yt-dlp: unexpected id shape "${id}"`));
      }
      resolve(id);
    });
  });
}

/**
 * Resolve (artist, title) → YouTube videoId. Returns null on lookup
 * failure (no result, yt-dlp crash, geo-block, etc.). All errors are
 * swallowed — the caller just drops the track. Misses are cached so
 * a broken search doesn't rerun on every refresh.
 */
async function searchMusicVideo(artist, title) {
  const key = cacheKey(artist, title);
  try {
    const hit = await redis.get(key);
    if (hit === NEGATIVE_SENTINEL) return null;
    if (hit) return hit;
  } catch (_) {}

  // Query : the official-video bias matters a lot — without the
  // "official" hint, YouTube surfaces fan uploads with lower quality
  // + bad metadata. "audio" fallback for tracks that never got a
  // video at all ends up ranked nearly as high by YouTube on its own.
  const query = `${artist} ${title} official`;
  let videoId = null;
  try {
    videoId = await spawnYtdlpSearch(query);
  } catch (err) {
    log.warn({ artist, title, err: err.message }, 'music-search: failed');
  }

  try {
    if (videoId) {
      await redis.set(key, videoId, 'EX', CACHE_TTL_HIT_SECS);
    } else {
      await redis.set(key, NEGATIVE_SENTINEL, 'EX', CACHE_TTL_MISS_SECS);
    }
  } catch (_) {}

  return videoId;
}

/**
 * Resolve many tracks in parallel with a bounded worker pool. Returns
 * a Map<track, videoId> keyed on the original track objects (caller
 * stays in control of ordering + dedup). `tracks` shape :
 * `[{ artist, title }, ...]`.
 */
async function searchMany(tracks, concurrency = 4) {
  const out = new Map();
  let idx = 0;
  async function worker() {
    while (true) {
      const i = idx++;
      if (i >= tracks.length) return;
      const t = tracks[i];
      const id = await searchMusicVideo(t.artist, t.title);
      if (id) out.set(t, id);
    }
  }
  const workers = Array.from({ length: concurrency }, worker);
  await Promise.all(workers);
  return out;
}

module.exports = { searchMusicVideo, searchMany };

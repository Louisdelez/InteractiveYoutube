/**
 * Periodic URL pre-resolution for all channels.
 *
 * Iterates `config.CHANNELS` and asks `url-resolver` to refresh the
 * googlevideo.com token for each channel's CURRENT video (derived from
 * `tv.getTvState` — the same source of truth used by the HTTP + socket
 * paths). Runs once at boot, then every `URL_RESOLVER_INTERVAL_MS`.
 *
 * Why a setInterval and not BullMQ:
 *   - No persistence needed (Redis cache IS the persistence layer).
 *   - No complex retries — if one tick fails, the next one 30 min later
 *     retries naturally. Token TTL is 6 h so we have 12 chances.
 *   - Concurrency budget is tiny (2 parallel yt-dlp at most — keeps the
 *     worker machine responsive).
 */
const config = require('../config');
const log = require('./../services/logger').child({ component: 'url-resolver-worker' });
const { getTvState } = require('../services/tv');
const urlResolver = require('../services/url-resolver');

const INTERVAL_MS =
  parseInt(process.env.URL_RESOLVER_INTERVAL_MS) || 30 * 60 * 1000;
const CONCURRENCY = parseInt(process.env.URL_RESOLVER_CONCURRENCY) || 2;
const INITIAL_DELAY_MS =
  parseInt(process.env.URL_RESOLVER_INITIAL_DELAY_MS) || 5_000;

let timer = null;
let running = false;

async function resolveOneChannel(channel) {
  try {
    const state = getTvState(channel.id);
    if (!state || !state.videoId) return;
    await urlResolver.resolveAndCache(channel.id, state.videoId);
    log.debug({ channelId: channel.id, videoId: state.videoId }, 'resolved');
  } catch (err) {
    log.warn({ channelId: channel.id, err: err.message }, 'channel resolve failed');
  }
}

/**
 * Bounded-concurrency sweep: spin up CONCURRENCY workers pulling from
 * a shared queue. 48 channels × ~500 ms yt-dlp ≈ 24 s wall at c=2, all
 * four cores barely tickled.
 */
async function runOnce() {
  if (running) {
    log.warn('previous sweep still running, skipping');
    return;
  }
  running = true;
  const started = Date.now();
  try {
    const queue = [...config.CHANNELS];
    const workers = Array.from({ length: CONCURRENCY }, async () => {
      while (queue.length > 0) {
        const channel = queue.shift();
        if (!channel) break;
        await resolveOneChannel(channel);
      }
    });
    await Promise.all(workers);
    log.info(
      { channels: config.CHANNELS.length, durationMs: Date.now() - started },
      'sweep complete',
    );
  } catch (err) {
    log.error({ err: err.message }, 'sweep crashed');
  } finally {
    running = false;
  }
}

function start() {
  if (timer) return;
  // Fire once shortly after boot (not immediately — give playlists +
  // yt-dlp updater a moment to settle so the very first sweep doesn't
  // fight with initialization work).
  setTimeout(() => {
    runOnce().catch((err) => log.error({ err: err.message }, 'initial run failed'));
  }, INITIAL_DELAY_MS);
  timer = setInterval(() => {
    runOnce().catch((err) => log.error({ err: err.message }, 'tick crashed'));
  }, INTERVAL_MS);
  log.info({ intervalMin: INTERVAL_MS / 60000, concurrency: CONCURRENCY }, 'url-resolver started');
}

function stop() {
  if (timer) { clearInterval(timer); timer = null; }
}

module.exports = { start, stop, runOnce };

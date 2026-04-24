const fs = require('fs');
const path = require('path');
const { fetchAllVideoIds, fetchOrderedVideoIds, fetchVideoDetails } = require('./youtube');
const config = require('../config');
const log = require('./logger').child({ component: 'playlist' });

const DATA_DIR = path.join(__dirname, '..', 'data');

// Store playlists per channel: { channelId: playlistState }
const playlists = new Map();
// Per-channel refresh lock — keyed by channelId, value is the in-flight
// Promise. Concurrent calls for the SAME channel return the existing
// promise; concurrent calls for DIFFERENT channels run in parallel.
// (The previous global boolean lock made any second concurrent
// refreshPlaylist call silently no-op even for a different channel.)
const refreshLocks = new Map();

// Mulberry32 seeded PRNG
function mulberry32(seed) {
  return function () {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t ^= t + Math.imul(t ^ (t >>> 7), 61 | t);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function seededShuffle(array, seed) {
  const rng = mulberry32(seed);
  const result = [...array];
  for (let i = result.length - 1; i > 0; i--) {
    const j = Math.floor(rng() * (i + 1));
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
}

function buildPrefixSums(videos) {
  const sums = new Float64Array(videos.length);
  sums[0] = videos[0].duration;
  for (let i = 1; i < videos.length; i++) {
    sums[i] = sums[i - 1] + videos[i].duration;
  }
  return sums;
}

function getPlaylistPath(channelId) {
  return path.join(DATA_DIR, `playlist-${channelId}.json`);
}

function saveToDisk(channelId) {
  const state = playlists.get(channelId);
  if (!state) return;
  try {
    const data = JSON.stringify(state, null, 2);
    const filePath = getPlaylistPath(channelId);
    const tmpPath = filePath + '.tmp';
    // Atomic write: write to temp file, then rename (rename is atomic on Linux)
    fs.writeFile(tmpPath, data, (err) => {
      if (err) {
        log.error({ channelId, err: err.message }, 'save: writeFile failed');
        return;
      }
      fs.rename(tmpPath, filePath, (err) => {
        if (err) log.error({ channelId, err: err.message }, 'save: rename failed');
        else log.debug({ channelId, videos: state.videos.length }, 'saved to disk');
      });
    });
  } catch (err) {
    log.error({ channelId, err: err.message }, 'save: serialize error');
  }
}

function loadFromDisk(channelId) {
  const filePath = getPlaylistPath(channelId);
  if (fs.existsSync(filePath)) {
    const state = JSON.parse(fs.readFileSync(filePath, 'utf-8'));
    state.prefixSums = buildPrefixSums(state.videos);
    playlists.set(channelId, state);
    log.info({ channelId, videos: state.videos.length }, 'loaded from disk');
    return true;
  }
  return false;
}

/**
 * Reload a single channel's state from disk. Used by the web to pick
 * up changes the worker has just persisted (pub/sub event
 * `koala:playlist-reload`). Silent no-op if the file doesn't exist
 * yet (first-boot race).
 */
function reloadFromDisk(channelId) {
  return loadFromDisk(channelId);
}

async function buildPlaylist(channelId, channel) {
  log.info({ channelId, kind: channel.kind }, 'building playlist');
  const allVideoIds = await channel.fetchVideoIds();
  log.info({ channelId, uniqueIds: allVideoIds.length }, 'collected video IDs');
  const videos = await fetchVideoDetails(allVideoIds, channel.detailsOpts);

  if (videos.length === 0) {
    throw new Error(`No embeddable videos found for channel ${channelId}`);
  }

  // Normal channels get a seeded shuffle so the order varies run-to-run
  // but each playback session is deterministic against its `seed`.
  // Ordered channels keep the fetch order (curated intent).
  const state = channel.shuffle
    ? buildShuffledState(channelId, videos, channel)
    : buildOrderedState(channelId, videos);

  playlists.set(channelId, state);
  saveToDisk(channelId);
  return state;
}

function buildShuffledState(channelId, videos, channel) {
  const seed = Date.now();
  const shuffled = seededShuffle(videos, seed);
  return {
    tvStartedAt: Date.now(),
    seed,
    totalDuration: shuffled.reduce((sum, v) => sum + v.duration, 0),
    channelId,
    youtubeChannelIds: channel.youtubeChannelIds,
    videos: shuffled,
    prefixSums: buildPrefixSums(shuffled),
    lastRefresh: Date.now(),
  };
}

function buildOrderedState(channelId, videos) {
  return {
    tvStartedAt: Date.now(),
    totalDuration: videos.reduce((sum, v) => sum + v.duration, 0),
    channelId,
    ordered: true,
    videos,
    prefixSums: buildPrefixSums(videos),
    lastRefresh: Date.now(),
  };
}

async function initAllPlaylists() {
  for (const channel of config.CHANNELS) {
    if (!loadFromDisk(channel.id)) {
      try {
        await buildPlaylist(channel.id, channel);
      } catch (err) {
        log.error({ channelId: channel.id, err: err.message }, 'initial build failed (will retry on next refresh)');
      }
    }
  }
}

// Append-only merge that preserves timecode across refreshes and restarts.
//
// TV position is `(now - tvStartedAt) % totalDuration`. If we reshuffle or
// regenerate videos on refresh, the viewer sees a jump. Instead:
//   1. Keep every old video in its exact slot (same order, same duration,
//      same metadata) → prefixSums up to oldLen are bit-identical.
//   2. Append truly-new videos at the end.
//   3. Rebase tvStartedAt so the *cycle-relative* position is preserved
//      even after totalDuration grows (matters once the TV has cycled).
function mergePlaylistPreservingTimecode(oldState, freshVideoDetails) {
  const oldIds = new Set(oldState.videos.map((v) => v.videoId));
  const trulyNew = freshVideoDetails.filter((v) => !oldIds.has(v.videoId));
  if (trulyNew.length === 0) {
    oldState.lastRefresh = Date.now();
    return { state: oldState, added: 0 };
  }

  const now = Date.now();
  const oldTotal = oldState.totalDuration;
  const elapsedInCycle = ((now - oldState.tvStartedAt) / 1000) % oldTotal;

  const merged = oldState.videos.concat(trulyNew);
  const totalDuration = merged.reduce((s, v) => s + v.duration, 0);

  // Set tvStartedAt so `(now - tvStartedAt) % totalDuration === elapsedInCycle`.
  const tvStartedAt = now - Math.floor(elapsedInCycle * 1000);

  const newState = {
    ...oldState,
    tvStartedAt,
    videos: merged,
    totalDuration,
    prefixSums: buildPrefixSums(merged),
    lastRefresh: now,
  };
  return { state: newState, added: trulyNew.length };
}

async function fetchFreshVideoIdsForChannel(channel) {
  // `fetchVideoIds` on the polymorphic channel object already handles
  // the per-kind dispatch (shuffle channel uploads for normal, playlist
  // concatenation for ordered, fixed array for fixed-video).
  return await channel.fetchVideoIds();
}

async function refreshPlaylist(channelId) {
  // De-duplicate concurrent calls for the SAME channel — return the
  // in-flight Promise. Different channels run in parallel.
  const inflight = refreshLocks.get(channelId);
  if (inflight) {
    log.debug({ channelId }, 'refresh already in progress, awaiting');
    return inflight;
  }

  const promise = (async () => {
    try {
      const channel = config.CHANNELS.find((c) => c.id === channelId);
      if (!channel) throw new Error(`Unknown channel: ${channelId}`);

      const oldState = playlists.get(channelId);

      // No prior state → bootstrap build (original path: new shuffle + start).
      if (!oldState) {
        log.info({ channelId }, 'full build (no prior state)');
        await buildPlaylist(channelId, channel);
        return playlists.get(channelId);
      }

      log.info({ channelId }, 'refresh (timecode-preserving)');
      const freshIds = await fetchFreshVideoIdsForChannel(channel);
      const knownIds = new Set(oldState.videos.map((v) => v.videoId));
      const trulyNewIds = freshIds.filter((id) => !knownIds.has(id));

      if (trulyNewIds.length === 0) {
        log.debug({ channelId }, 'refresh: no new videos, state unchanged');
        oldState.lastRefresh = Date.now();
        saveToDisk(channelId);
        return oldState;
      }

      const trulyNewVideos = await fetchVideoDetails(trulyNewIds, channel.detailsOpts);

      const { state: newState, added } = mergePlaylistPreservingTimecode(
        oldState, trulyNewVideos
      );
      playlists.set(channelId, newState);
      saveToDisk(channelId);
      log.info({ channelId, added, total: newState.videos.length }, 'refresh complete (timecode preserved)');
      return newState;
    } finally {
      refreshLocks.delete(channelId);
    }
  })();
  refreshLocks.set(channelId, promise);
  return promise;
}

function addNewVideos(channelId, newVideos) {
  const state = playlists.get(channelId);
  if (!state || newVideos.length === 0) return;

  const existingIds = new Set(state.videos.map((v) => v.videoId));
  const trulyNew = newVideos.filter((v) => !existingIds.has(v.videoId));
  if (trulyNew.length === 0) return;

  // Append + rebase tvStartedAt to preserve the cycle-relative position.
  // Without this, growing totalDuration shifts `(now-start) % total` for
  // future cycles — viewer sees a jump.
  const { state: next, added } = mergePlaylistPreservingTimecode(state, trulyNew);
  playlists.set(channelId, next);
  saveToDisk(channelId);
  log.info({ channelId, added }, 'added new videos (timecode preserved)');
}

function getPlaylist(channelId) {
  return playlists.get(channelId) || null;
}

function getVideoIds(channelId) {
  const state = playlists.get(channelId);
  if (!state) return [];
  return state.videos.map((v) => v.videoId);
}

module.exports = {
  initAllPlaylists,
  refreshPlaylist,
  addNewVideos,
  getPlaylist,
  getVideoIds,
  reloadFromDisk,
};

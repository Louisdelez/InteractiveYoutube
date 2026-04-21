const fs = require('fs');
const path = require('path');
const { fetchAllVideoIds, fetchOrderedVideoIds, fetchVideoDetails } = require('./youtube');
const config = require('../config');

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
        console.error(`[Playlist:${channelId}] Failed to save:`, err.message);
        return;
      }
      fs.rename(tmpPath, filePath, (err) => {
        if (err) console.error(`[Playlist:${channelId}] Failed to rename:`, err.message);
        else console.log(`[Playlist:${channelId}] Saved to disk (${state.videos.length} videos)`);
      });
    });
  } catch (err) {
    console.error(`[Playlist:${channelId}] Serialize error:`, err.message);
  }
}

function loadFromDisk(channelId) {
  const filePath = getPlaylistPath(channelId);
  if (fs.existsSync(filePath)) {
    const state = JSON.parse(fs.readFileSync(filePath, 'utf-8'));
    state.prefixSums = buildPrefixSums(state.videos);
    playlists.set(channelId, state);
    console.log(`[Playlist:${channelId}] Loaded from disk (${state.videos.length} videos)`);
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
  if (channel.ordered && channel.fixedVideoIds) {
    // Fixed video list (e.g., Noob: specific videos in order)
    console.log(`[Playlist:${channelId}] Building ORDERED from ${channel.fixedVideoIds.length} fixed videos...`);
    const allVideoIds = channel.fixedVideoIds;
    const videos = await fetchVideoDetails(allVideoIds, { skipShortsFilter: true, skipLiveFilter: true });

    if (videos.length === 0) {
      throw new Error(`No embeddable videos found for channel ${channelId}`);
    }

    const totalDuration = videos.reduce((sum, v) => sum + v.duration, 0);

    const state = {
      tvStartedAt: Date.now(),
      totalDuration,
      channelId,
      ordered: true,
      videos,
      prefixSums: buildPrefixSums(videos),
      lastRefresh: Date.now(),
    };

    playlists.set(channelId, state);
    saveToDisk(channelId);
    return state;
  }

  if (channel.ordered && channel.youtubePlaylists) {
    console.log(`[Playlist:${channelId}] Building ORDERED from ${channel.youtubePlaylists.length} playlists...`);
    const allVideoIds = await fetchOrderedVideoIds(channel.youtubePlaylists);
    // For ordered: don't filter shorts via HEAD (these are curated playlists)
    // But still fetch details for duration and embeddable check
    const videos = await fetchVideoDetails(allVideoIds, { skipShortsFilter: true, skipLiveFilter: true });

    if (videos.length === 0) {
      throw new Error(`No embeddable videos found for channel ${channelId}`);
    }

    const totalDuration = videos.reduce((sum, v) => sum + v.duration, 0);

    const state = {
      tvStartedAt: Date.now(),
      totalDuration,
      channelId,
      ordered: true,
      videos, // Keep original order, no shuffle
      prefixSums: buildPrefixSums(videos),
      lastRefresh: Date.now(),
    };

    playlists.set(channelId, state);
    saveToDisk(channelId);
    return state;
  }

  // Normal channel: fetch from YouTube channel uploads + shuffle
  const youtubeChannelIds = channel.youtubeChannelIds;
  console.log(`[Playlist:${channelId}] Building from YouTube (${youtubeChannelIds.length} source(s))...`);
  let allVideoIds = [];
  for (const ytId of youtubeChannelIds) {
    const ids = await fetchAllVideoIds(ytId);
    allVideoIds = allVideoIds.concat(ids);
  }
  // Also fetch from extra playlists (e.g., EGO's unlisted old videos)
  if (channel.extraPlaylists) {
    for (const plId of channel.extraPlaylists) {
      const ids = await fetchOrderedVideoIds([plId]);
      allVideoIds = allVideoIds.concat(ids);
    }
  }
  allVideoIds = [...new Set(allVideoIds)];
  console.log(`[Playlist:${channelId}] Total unique video IDs: ${allVideoIds.length}`);
  const videos = await fetchVideoDetails(allVideoIds, {
    skipLiveFilter: !!channel.includeLives,
  });

  if (videos.length === 0) {
    throw new Error(`No embeddable videos found for channel ${channelId}`);
  }

  const seed = Date.now();
  const shuffled = seededShuffle(videos, seed);
  const totalDuration = shuffled.reduce((sum, v) => sum + v.duration, 0);

  const state = {
    tvStartedAt: Date.now(),
    seed,
    totalDuration,
    channelId,
    youtubeChannelIds,
    videos: shuffled,
    prefixSums: buildPrefixSums(shuffled),
    lastRefresh: Date.now(),
  };

  playlists.set(channelId, state);
  saveToDisk(channelId);
  return state;
}

async function initAllPlaylists() {
  for (const channel of config.CHANNELS) {
    if (!loadFromDisk(channel.id)) {
      try {
        await buildPlaylist(channel.id, channel);
      } catch (err) {
        console.error(`[Playlist:${channel.id}] Build failed (will retry on next refresh): ${err.message}`);
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
  if (channel.ordered && channel.fixedVideoIds) {
    return channel.fixedVideoIds;
  }
  if (channel.ordered && channel.youtubePlaylists) {
    return await fetchOrderedVideoIds(channel.youtubePlaylists);
  }
  let ids = [];
  for (const ytId of channel.youtubeChannelIds || []) {
    ids = ids.concat(await fetchAllVideoIds(ytId));
  }
  if (channel.extraPlaylists) {
    for (const plId of channel.extraPlaylists) {
      ids = ids.concat(await fetchOrderedVideoIds([plId]));
    }
  }
  return [...new Set(ids)];
}

async function refreshPlaylist(channelId) {
  // De-duplicate concurrent calls for the SAME channel — return the
  // in-flight Promise. Different channels run in parallel.
  const inflight = refreshLocks.get(channelId);
  if (inflight) {
    console.log(`[Playlist:${channelId}] Refresh already in progress, awaiting it`);
    return inflight;
  }

  const promise = (async () => {
    try {
      const channel = config.CHANNELS.find((c) => c.id === channelId);
      if (!channel) throw new Error(`Unknown channel: ${channelId}`);

      const oldState = playlists.get(channelId);

      // No prior state → bootstrap build (original path: new shuffle + start).
      if (!oldState) {
        console.log(`[Playlist:${channelId}] Full build (no prior state)...`);
        await buildPlaylist(channelId, channel);
        return playlists.get(channelId);
      }

      console.log(`[Playlist:${channelId}] Refresh (timecode-preserving)...`);
      const freshIds = await fetchFreshVideoIdsForChannel(channel);
      const knownIds = new Set(oldState.videos.map((v) => v.videoId));
      const trulyNewIds = freshIds.filter((id) => !knownIds.has(id));

      if (trulyNewIds.length === 0) {
        console.log(`[Playlist:${channelId}] Refresh: no new videos, state unchanged`);
        oldState.lastRefresh = Date.now();
        saveToDisk(channelId);
        return oldState;
      }

      const trulyNewVideos = await fetchVideoDetails(trulyNewIds, {
        skipShortsFilter: !!channel.ordered,
        skipLiveFilter: !!channel.ordered || !!channel.includeLives,
      });

      const { state: newState, added } = mergePlaylistPreservingTimecode(
        oldState, trulyNewVideos
      );
      playlists.set(channelId, newState);
      saveToDisk(channelId);
      console.log(
        `[Playlist:${channelId}] Refresh: +${added} videos (total ${newState.videos.length}), timecode preserved`
      );
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
  console.log(`[Playlist:${channelId}] Added ${added} new videos (timecode preserved)`);
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

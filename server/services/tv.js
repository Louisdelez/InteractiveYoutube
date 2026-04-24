const { getPlaylist } = require('./playlist');
const log = require('./logger').child({ component: 'tv' });
const urlResolver = require('./url-resolver');

// Per-channel cache and priority queues
const channelCache = new Map(); // channelId -> { state, cachedAt }
const priorityQueues = new Map(); // channelId -> []
const lastVideoIds = new Map(); // channelId -> videoId
const priorityState = new Map(); // channelId -> { playing, startedAt }

const CACHE_TTL_MS = parseInt(process.env.TV_STATE_CACHE_TTL_MS) || 1000;

function queuePriorityVideo(channelId, video) {
  if (!priorityQueues.has(channelId)) priorityQueues.set(channelId, []);
  const queue = priorityQueues.get(channelId);
  if (queue.some((v) => v.videoId === video.videoId)) return;
  queue.push(video);
  channelCache.delete(channelId);
  // Drop the cached resolved URL for this channel — it belongs to the
  // previous video and would ship the wrong stream to clients. The
  // next url-resolver sweep will re-populate it for the new videoId.
  urlResolver.invalidate(channelId).catch(() => {});
  log.info({ channelId, title: video.title }, 'priority queued');
}

/**
 * Merge the Redis-cached pre-resolved URLs into a tv:state payload.
 * Cache is only applied if the cached videoId matches `state.videoId`
 * exactly — protects against serving a URL from the previous video
 * after an auto-advance or priority injection.
 *
 * Stays best-effort: any Redis error just returns `state` unchanged
 * and lets mpv fall back to its normal ytdl_hook path.
 */
async function enrichWithResolvedUrl(state) {
  if (!state || !state.videoId) return state;
  try {
    const cached = await urlResolver.getCached(state.channelId, state.videoId);
    if (!cached) return state;
    return {
      ...state,
      resolvedUrl: cached.mainUrl || undefined,
      resolvedUrlLq: cached.lqUrl || undefined,
      resolvedAt: cached.resolvedAt,
    };
  } catch (_) {
    return state;
  }
}

function getTvState(channelId) {
  const now = Date.now();

  const playlist = getPlaylist(channelId);
  if (!playlist || !playlist.videos.length) {
    return null;
  }

  const { tvStartedAt, totalDuration, videos, prefixSums } = playlist;

  // Normal rotation
  const elapsedSec = ((now - tvStartedAt) / 1000) % totalDuration;

  let normalIndex = 0;
  let normalSeekTo = 0;

  if (prefixSums) {
    let lo = 0;
    let hi = videos.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (prefixSums[mid] <= elapsedSec) lo = mid + 1;
      else hi = mid;
    }
    normalIndex = lo;
    const accumulated = lo > 0 ? prefixSums[lo - 1] : 0;
    normalSeekTo = elapsedSec - accumulated;
  } else {
    let accumulated = 0;
    for (let i = 0; i < videos.length; i++) {
      if (accumulated + videos[i].duration > elapsedSec) {
        normalIndex = i;
        normalSeekTo = elapsedSec - accumulated;
        break;
      }
      accumulated += videos[i].duration;
    }
  }

  const normalVideoId = videos[normalIndex].videoId;
  const queue = priorityQueues.get(channelId) || [];
  const pState = priorityState.get(channelId) || { playing: false, startedAt: null };

  // Helper: compute the upcoming video for the normal rotation given
  // the current normalIndex.
  function nextNormal() {
    const nextIdx = (normalIndex + 1) % videos.length;
    const v = videos[nextIdx];
    return { id: v.videoId, title: v.title, duration: v.duration };
  }

  // Currently playing priority video
  if (pState.playing && pState.startedAt) {
    const priorityElapsed = (now - pState.startedAt) / 1000;
    const priorityVideo = queue[0];

    if (priorityVideo && priorityElapsed < priorityVideo.duration) {
      // After the priority video, next is either the next priority in queue
      // or the normal-rotation video at the moment priority ends.
      const next = queue[1]
        ? { id: queue[1].videoId, title: queue[1].title, duration: queue[1].duration }
        : nextNormal();
      return {
        videoId: priorityVideo.videoId,
        title: priorityVideo.title,
        videoIndex: -1,
        seekTo: priorityElapsed,
        duration: priorityVideo.duration,
        embeddable: priorityVideo.embeddable !== false,
        publishedAt: priorityVideo.publishedAt || null,
        serverTime: now,
        totalVideos: videos.length,
        channelId,
        isPriority: true,
        nextVideoId: next.id,
        nextTitle: next.title,
        nextDuration: next.duration,
      };
    }

    // Priority finished
    queue.shift();
    pState.playing = false;
    pState.startedAt = null;
    priorityState.set(channelId, pState);
    channelCache.delete(channelId);
    log.info({ channelId }, 'priority video finished, resuming rotation');
  }

  // Detect video change → inject priority
  const lastVid = lastVideoIds.get(channelId);
  if (lastVid && lastVid !== normalVideoId && queue.length > 0 && !pState.playing) {
    pState.playing = true;
    pState.startedAt = now;
    priorityState.set(channelId, pState);
    const priorityVideo = queue[0];
    lastVideoIds.set(channelId, priorityVideo.videoId);
    log.info({ channelId, title: priorityVideo.title }, 'playing priority video');

    const next = queue[1]
      ? { id: queue[1].videoId, title: queue[1].title, duration: queue[1].duration }
      : nextNormal();
    return {
      videoId: priorityVideo.videoId,
      title: priorityVideo.title,
      videoIndex: -1,
      seekTo: 0,
      duration: priorityVideo.duration,
      embeddable: priorityVideo.embeddable !== false,
      publishedAt: priorityVideo.publishedAt || null,
      serverTime: now,
      totalVideos: videos.length,
      channelId,
      isPriority: true,
      nextVideoId: next.id,
      nextTitle: next.title,
      nextDuration: next.duration,
    };
  }

  // Auto-advance detection: when the playlist rotation lands on a new
  // video vs. what we remembered, the URL cache for this channel is
  // stale (it was keyed to the previous video). Fire-and-forget a
  // fresh resolve so the NEXT tv:state request serves a warm URL
  // instead of falling through to ytdl_hook on the client. Without
  // this, zap hit rate drops to ~50% between the 30-min sweeps
  // (every ~5-15 min of video → every other click misses cache).
  const prevVideoId = lastVideoIds.get(channelId);
  if (prevVideoId && prevVideoId !== normalVideoId) {
    urlResolver.invalidate(channelId).catch(() => {});
    // Schedule the re-resolve on next tick so we don't block the
    // getTvState caller (this function runs on the hot socket path).
    setImmediate(() => {
      urlResolver
        .resolveAndCache(channelId, normalVideoId)
        .catch((err) => log.warn(
          { channelId, videoId: normalVideoId, err: err.message },
          'auto-advance resolve failed',
        ));
    });
  }
  lastVideoIds.set(channelId, normalVideoId);

  // Return cached
  const cached = channelCache.get(channelId);
  if (cached && now - cached.cachedAt < CACHE_TTL_MS) {
    return { ...cached.state, serverTime: now };
  }

  const next = nextNormal();
  const state = {
    videoId: normalVideoId,
    title: videos[normalIndex].title,
    videoIndex: normalIndex,
    seekTo: normalSeekTo,
    duration: videos[normalIndex].duration,
    embeddable: videos[normalIndex].embeddable !== false,
    publishedAt: videos[normalIndex].publishedAt || null,
    serverTime: now,
    totalVideos: videos.length,
    channelId,
    isPriority: false,
    nextVideoId: next.id,
    nextTitle: next.title,
    nextDuration: next.duration,
  };

  channelCache.set(channelId, { state, cachedAt: now });
  return { ...state, serverTime: now };
}

module.exports = { getTvState, queuePriorityVideo, enrichWithResolvedUrl };

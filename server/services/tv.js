const { getPlaylist } = require('./playlist');

// Per-channel cache and priority queues
const channelCache = new Map(); // channelId -> { state, cachedAt }
const priorityQueues = new Map(); // channelId -> []
const lastVideoIds = new Map(); // channelId -> videoId
const priorityState = new Map(); // channelId -> { playing, startedAt }

const CACHE_TTL_MS = 1000;

function queuePriorityVideo(channelId, video) {
  if (!priorityQueues.has(channelId)) priorityQueues.set(channelId, []);
  const queue = priorityQueues.get(channelId);
  if (queue.some((v) => v.videoId === video.videoId)) return;
  queue.push(video);
  channelCache.delete(channelId);
  console.log(`[TV:${channelId}] Priority queued: "${video.title}"`);
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
    console.log(`[TV:${channelId}] Priority video finished, resuming rotation`);
  }

  // Detect video change → inject priority
  const lastVid = lastVideoIds.get(channelId);
  if (lastVid && lastVid !== normalVideoId && queue.length > 0 && !pState.playing) {
    pState.playing = true;
    pState.startedAt = now;
    priorityState.set(channelId, pState);
    const priorityVideo = queue[0];
    lastVideoIds.set(channelId, priorityVideo.videoId);
    console.log(`[TV:${channelId}] Playing priority: "${priorityVideo.title}"`);

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
      serverTime: now,
      totalVideos: videos.length,
      channelId,
      isPriority: true,
      nextVideoId: next.id,
      nextTitle: next.title,
      nextDuration: next.duration,
    };
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
    embeddable: videos[normalIndex].embeddable !== false, // backward compat: undefined = true
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

module.exports = { getTvState, queuePriorityVideo };

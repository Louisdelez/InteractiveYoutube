const { google } = require('googleapis');
const https = require('https');
const fs = require('fs');
const path = require('path');
const config = require('../config');
const log = require('./logger').child({ component: 'youtube' });

const youtube = google.youtube({
  version: 'v3',
  auth: config.YOUTUBE_API_KEY,
});

// Persistent on-disk cache of "is this video a YouTube Short?" so
// we don't re-probe YouTube on every refresh. Shorts status is
// effectively immutable (once a Short, always a Short).
const SHORTS_CACHE_PATH = path.join(__dirname, '..', 'data', 'shorts-cache.json');
let shortsCache = {};
try {
  if (fs.existsSync(SHORTS_CACHE_PATH)) {
    shortsCache = JSON.parse(fs.readFileSync(SHORTS_CACHE_PATH, 'utf8'));
  }
} catch (err) {
  shortsCache = {};
}
let shortsCacheDirty = false;
function flushShortsCache() {
  if (!shortsCacheDirty) return;
  try {
    fs.writeFileSync(SHORTS_CACHE_PATH, JSON.stringify(shortsCache));
    shortsCacheDirty = false;
  } catch (_) {}
}
// Best-effort flush on graceful shutdown
process.once('beforeExit', flushShortsCache);

// Convert ISO 8601 duration (PT1H2M3S) to seconds
function parseDuration(iso) {
  const match = iso.match(/PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?/);
  if (!match) return 0;
  const h = parseInt(match[1] || 0);
  const m = parseInt(match[2] || 0);
  const s = parseInt(match[3] || 0);
  return h * 3600 + m * 60 + s;
}

// Check if a video is a YouTube Short via HEAD request.
// Returns true if it's a Short (HTTP 200), false otherwise.
// Cached on disk + jittered + identified UA to reduce IP rate-limits.
const UA = 'Mozilla/5.0 (X11; Linux x86_64) IY-PlaylistBuilder/1.0';
function checkIsShort(videoId) {
  if (Object.prototype.hasOwnProperty.call(shortsCache, videoId)) {
    return Promise.resolve(shortsCache[videoId]);
  }
  return new Promise((resolve) => {
    // 0-200ms jitter to avoid bursts
    setTimeout(() => {
      const req = https.request(
        `https://www.youtube.com/shorts/${videoId}`,
        {
          method: 'HEAD',
          timeout: 5000,
          headers: { 'User-Agent': UA },
        },
        (res) => {
          const isShort = res.statusCode === 200;
          shortsCache[videoId] = isShort;
          shortsCacheDirty = true;
          resolve(isShort);
        }
      );
      req.on('error', () => resolve(false));
      req.on('timeout', () => { req.destroy(); resolve(false); });
      req.end();
    }, Math.random() * 200);
  });
}

// Check multiple videos in parallel with concurrency limit
async function filterOutShorts(videos, concurrency = 20) {
  const results = [];
  let shortsCount = 0;

  for (let i = 0; i < videos.length; i += concurrency) {
    const batch = videos.slice(i, i + concurrency);
    const checks = await Promise.all(
      batch.map(async (video) => {
        const isShort = await checkIsShort(video.videoId);
        return { video, isShort };
      })
    );

    for (const { video, isShort } of checks) {
      if (isShort) {
        shortsCount++;
      } else {
        results.push(video);
      }
    }

    if ((i + concurrency) % 200 === 0 || i + concurrency >= videos.length) {
      log.debug({ checked: Math.min(i + concurrency, videos.length), total: videos.length, shortsCount }, 'shorts filter progress');
    }
  }

  log.info({ shortsFiltered: shortsCount, remaining: results.length }, 'shorts filter done');
  // Persist whatever we learned to the disk cache.
  flushShortsCache();
  return results;
}

// Get the uploads playlist ID for a channel
async function getUploadsPlaylistId(channelId) {
  const res = await youtube.channels.list({
    part: 'contentDetails',
    id: channelId,
  });

  if (!res.data.items || res.data.items.length === 0) {
    throw new Error(`Channel not found: ${channelId}`);
  }

  return res.data.items[0].contentDetails.relatedPlaylists.uploads;
}

// Fetch all video IDs from the uploads playlist
async function fetchAllVideoIds(channelId) {
  const uploadsPlaylistId = await getUploadsPlaylistId(channelId);
  const videoIds = [];
  let pageToken = undefined;

  log.debug({ uploadsPlaylistId }, 'fetching videos from uploads playlist');

  do {
    const res = await youtube.playlistItems.list({
      part: 'contentDetails',
      playlistId: uploadsPlaylistId,
      maxResults: 50,
      pageToken,
    });

    for (const item of res.data.items) {
      videoIds.push(item.contentDetails.videoId);
    }

    pageToken = res.data.nextPageToken;
    log.trace({ soFar: videoIds.length }, 'fetching more pages');
  } while (pageToken);

  log.info({ uploadsPlaylistId, videoIds: videoIds.length }, 'fetched all videos from uploads');
  return videoIds;
}

// Fetch video IDs from a specific playlist (keeps order)
async function fetchPlaylistVideoIds(playlistId) {
  const videoIds = [];
  let pageToken = undefined;

  do {
    const res = await youtube.playlistItems.list({
      part: 'contentDetails',
      playlistId,
      maxResults: 50,
      pageToken,
    });

    for (const item of res.data.items) {
      videoIds.push(item.contentDetails.videoId);
    }

    pageToken = res.data.nextPageToken;
  } while (pageToken);

  return videoIds;
}

// Fetch video IDs from multiple playlists in order (for ordered channels like Popcorn)
async function fetchOrderedVideoIds(playlistIds) {
  const allIds = [];
  for (let i = 0; i < playlistIds.length; i++) {
    const ids = await fetchPlaylistVideoIds(playlistIds[i]);
    log.debug({ playlist: i + 1, of: playlistIds.length, videos: ids.length }, 'fetched playlist');
    allIds.push(...ids);
  }
  log.info({ totalOrderedIds: allIds.length }, 'fetched ordered playlists');
  return allIds;
}

// Fetch video details (duration, embeddable) in batches of 50
async function fetchVideoDetails(videoIds, options = {}) {
  const videos = [];
  let livesFiltered = 0;

  for (let i = 0; i < videoIds.length; i += 50) {
    const batch = videoIds.slice(i, i + 50);
    const res = await youtube.videos.list({
      part: 'contentDetails,status,snippet,liveStreamingDetails',
      id: batch.join(','),
    });

    for (const item of res.data.items) {
      const duration = parseDuration(item.contentDetails.duration);
      const isLive = !!item.liveStreamingDetails;
      const isLiveNow = item.snippet.liveBroadcastContent === 'live' || item.snippet.liveBroadcastContent === 'upcoming';

      // Skip: duration 0, live streams and live replays
      // Include non-embeddable videos with a flag (for Tauri app support)
      // skipLiveFilter: for curated/fixed playlists where lives are intentionally included
      const filterLive = !options.skipLiveFilter && (isLive || isLiveNow);
      if (duration > 0 && !filterLive) {
        videos.push({
          videoId: item.id,
          title: item.snippet.title,
          duration,
          embeddable: !!item.status.embeddable,
          publishedAt: item.snippet.publishedAt || null,
        });
      } else if (isLive) {
        livesFiltered++;
      }
    }

    log.trace({ fetched: Math.min(i + 50, videoIds.length), total: videoIds.length }, 'video details batch done');
  }

  if (livesFiltered > 0) {
    log.info({ livesFiltered }, 'filtered out live streams / replays');
  }
  log.info({ videos: videos.length }, 'embeddable videos (before shorts filter)');

  // Filter out YouTube Shorts (skip for ordered/curated playlists)
  if (options.skipShortsFilter) {
    return videos;
  }
  const filtered = await filterOutShorts(videos);
  return filtered;
}

module.exports = { fetchAllVideoIds, fetchOrderedVideoIds, fetchVideoDetails, parseDuration };

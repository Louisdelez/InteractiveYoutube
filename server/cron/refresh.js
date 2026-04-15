const cron = require('node-cron');
const config = require('../config');
const { refreshPlaylist, addNewVideos } = require('../services/playlist');
const { fetchVideoDetails } = require('../services/youtube');
const { checkForNewUploads } = require('../services/rss');
const { queuePriorityVideo } = require('../services/tv');
const { getIO } = require('../socket');
const { clearAllChatHistory } = require('../socket/chat');

// Minimum duration (in seconds) for a Popcorn episode to be considered a full episode
const POPCORN_MIN_DURATION = 5400; // 1h30

function startCronJobs() {
  // Daily 3am: refresh a 1/7th slice of the channels, then restart the
  // process. Splits the full-refresh load across the week (API-friendly)
  // and guarantees a fresh process every morning (clears leaks, reloads
  // config.js edits). The process manager (PM2 / nodemon) respawns us.
  cron.schedule(config.DAILY_REFRESH_CRON, async () => {
    const day = new Date().getDay(); // 0 = Sun … 6 = Sat
    const bucket = config.CHANNELS.filter((_, i) => i % 7 === day);
    console.log(
      `[CRON] Daily refresh: day=${day}, ${bucket.length}/${config.CHANNELS.length} channels`
    );
    for (const channel of bucket) {
      console.log(`[CRON] Daily refresh: ${channel.id}`);
      try {
        await refreshPlaylist(channel.id);
        const io = getIO();
        if (io) {
          io.to(`channel:${channel.id}`).emit('tv:refreshed');
        }
      } catch (err) {
        console.error(`[CRON] Daily refresh failed for ${channel.id}:`, err.message);
      }
    }
    // Wipe chat history across all channels and notify clients.
    try {
      const cleared = await clearAllChatHistory(getIO());
      console.log(`[CRON] Chat cleared: ${cleared} channel histories wiped`);
    } catch (err) {
      console.error('[CRON] Chat clear failed:', err.message);
    }
    console.log('[CRON] Daily batch complete — restarting process');
    // Exit 1 so *both* PM2 (autorestart) and nodemon (treats non-zero as
    // crash → respawn) bring us back up. Small delay lets sockets/logs flush.
    setTimeout(() => process.exit(1), 2000);
  });

  console.log(`[CRON] Daily refresh scheduled: ${config.DAILY_REFRESH_CRON}`);

  // RSS poll every 30 minutes for all channels
  setInterval(async () => {
    for (const channel of config.CHANNELS) {
      try {
        if (channel.ordered) {
          // Ordered channels (like Popcorn): check RSS for new full episodes
          await pollOrderedChannel(channel);
        } else {
          // Normal channels: check RSS for any new videos
          await pollNormalChannel(channel);
        }
      } catch (err) {
        console.error(`[RSS:${channel.id}] Poll error:`, err.message);
      }
    }
  }, config.RSS_POLL_INTERVAL_MS);

  console.log(`[CRON] RSS poll started (every ${config.RSS_POLL_INTERVAL_MS / 60000} min)`);
}

async function pollNormalChannel(channel) {
  let newIds = [];
  for (const ytId of channel.youtubeChannelIds) {
    const ids = await checkForNewUploads(channel.id, ytId);
    newIds = newIds.concat(ids);
  }
  newIds = [...new Set(newIds)];

  if (newIds.length > 0) {
    const newVideos = await fetchVideoDetails(newIds);
    if (newVideos.length > 0) {
      for (const video of newVideos) {
        queuePriorityVideo(channel.id, video);
      }
      addNewVideos(channel.id, newVideos);

      const io = getIO();
      if (io) {
        io.to(`channel:${channel.id}`).volatile.emit('tv:newRelease', {
          videos: newVideos.map((v) => ({ title: v.title, videoId: v.videoId })),
        });
      }

      console.log(`[RSS:${channel.id}] ${newVideos.length} new video(s) queued`);
    }
  }
}

async function pollOrderedChannel(channel) {
  // Check the Popcorn YouTube channel uploads RSS for new videos
  const popcornYtChannelId = 'UCnyR4T5qpgOrWGcQU6Jinkw';
  const newIds = await checkForNewUploads(channel.id, popcornYtChannelId);

  if (newIds.length === 0) return;

  // Fetch video details (skip shorts filter for ordered channels)
  const newVideos = await fetchVideoDetails(newIds, { skipShortsFilter: true });

  // Filter: only keep full Popcorn episodes
  // Criteria: title contains "POPCORN" (case insensitive) AND duration > 1h30
  const popcornEpisodes = newVideos.filter((v) => {
    const titleMatch = /popcorn/i.test(v.title);
    const longEnough = v.duration >= POPCORN_MIN_DURATION;
    return titleMatch && longEnough;
  });

  if (popcornEpisodes.length === 0) return;

  // Add to the end of the ordered playlist (not random, keeps order)
  addNewVideos(channel.id, popcornEpisodes);

  const io = getIO();
  if (io) {
    io.to(`channel:${channel.id}`).volatile.emit('tv:newRelease', {
      videos: popcornEpisodes.map((v) => ({ title: v.title, videoId: v.videoId })),
    });
  }

  console.log(`[RSS:${channel.id}] ${popcornEpisodes.length} new Popcorn episode(s) added: ${popcornEpisodes.map(v => v.title).join(', ')}`);
}

module.exports = { startCronJobs };

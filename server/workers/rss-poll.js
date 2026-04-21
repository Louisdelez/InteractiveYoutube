/**
 * RSS polling — detects newly published videos every RSS_POLL_INTERVAL_MS
 * and injects them as priority videos for the relevant channel.
 *
 * Moved from server/cron/refresh.js into the worker process so that
 * nodemon watching server/data/*.json (in dev) no longer restarts the
 * web mid-poll, and so the web server stays pure HTTP/WS with no
 * setInterval-based mutations.
 *
 * State-crossing: the poll mutates the worker's in-memory playlist
 * state and writes JSON to disk, then publishes two pub/sub events so
 * the web server can update its own in-memory mirror:
 *   - koala:playlist-reload  → web reloads the JSON from disk
 *   - koala:priority-video   → web appends to its priority queue
 *
 * Socket.IO fan-out to end clients still goes through the redis
 * adapter — the worker emits via @socket.io/redis-emitter and clients
 * receive those events without any web-side forwarding.
 */
const config = require('../config');
const log = require('../services/logger');
const { fetchVideoDetails } = require('../services/youtube');
const { checkForNewUploads } = require('../services/rss');
const { addNewVideos } = require('../services/playlist');
const { getEmitter } = require('./io-emitter');
const pubsub = require('../services/pubsub');

const POPCORN_MIN_DURATION = 5400; // 1 h 30 (same as the old cron)

async function pollNormalChannel(channel) {
  let newIds = [];
  for (const ytId of channel.youtubeChannelIds) {
    const ids = await checkForNewUploads(channel.id, ytId);
    newIds = newIds.concat(ids);
  }
  newIds = [...new Set(newIds)];
  if (newIds.length === 0) return;

  const newVideos = await fetchVideoDetails(newIds);
  if (newVideos.length === 0) return;

  addNewVideos(channel.id, newVideos);
  await pubsub.publish(pubsub.CHANNELS.PLAYLIST_RELOAD, { channelId: channel.id });

  const emitter = getEmitter();
  for (const video of newVideos) {
    await pubsub.publish(pubsub.CHANNELS.PRIORITY_VIDEO, { channelId: channel.id, video });
  }
  emitter.emit('tv:playlistUpdated', { channelId: channel.id });
  emitter.emit('tv:newRelease', { channelId: channel.id, count: newVideos.length });

  log.info({ channel: channel.id, count: newVideos.length }, 'rss: new videos injected');
}

async function pollOrderedChannel(channel) {
  // Only Popcorn is currently ordered-with-RSS. Fixed-video channels
  // (noob) never get RSS treatment — their playlist is static.
  const popcornYtChannelId = 'UCnyR4T5qpgOrWGcQU6Jinkw';
  const newIds = await checkForNewUploads(channel.id, popcornYtChannelId);
  if (newIds.length === 0) return;

  const newVideos = await fetchVideoDetails(newIds, { skipShortsFilter: true });
  const popcornEpisodes = newVideos.filter(
    (v) => /popcorn/i.test(v.title) && v.duration >= POPCORN_MIN_DURATION
  );
  if (popcornEpisodes.length === 0) return;

  addNewVideos(channel.id, popcornEpisodes);
  await pubsub.publish(pubsub.CHANNELS.PLAYLIST_RELOAD, { channelId: channel.id });

  const emitter = getEmitter();
  emitter.emit('tv:playlistUpdated', { channelId: channel.id });
  emitter.emit('tv:newRelease', { channelId: channel.id, count: popcornEpisodes.length });

  log.info({ channel: channel.id, count: popcornEpisodes.length }, 'rss: popcorn episodes added');
}

async function pollOnce() {
  for (const channel of config.CHANNELS) {
    try {
      if (channel.ordered && channel.youtubePlaylists) {
        await pollOrderedChannel(channel);
      } else if (!channel.ordered) {
        await pollNormalChannel(channel);
      }
      // Fixed-ordered channels (noob) intentionally have no RSS
    } catch (err) {
      log.error({ channel: channel.id, err: err.message }, 'rss poll: channel failed');
    }
  }
}

let timer = null;
function start() {
  if (timer) return;
  timer = setInterval(() => {
    pollOnce().catch((err) => log.error({ err: err.message }, 'rss poll tick crashed'));
  }, config.RSS_POLL_INTERVAL_MS);
  log.info({ intervalMin: config.RSS_POLL_INTERVAL_MS / 60000 }, 'rss poll started');
}

function stop() {
  if (timer) { clearInterval(timer); timer = null; }
}

module.exports = { start, stop, pollOnce };

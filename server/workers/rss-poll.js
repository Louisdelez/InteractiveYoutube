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
const { addNewVideos } = require('../services/playlist');
const { getEmitter } = require('./io-emitter');
const pubsub = require('../services/pubsub');

/**
 * Poll one channel for new videos via its RSS feed. Each channel
 * implements `pollRss()` polymorphically:
 *   - NormalChannel          : uploads RSS for each underlying YouTube
 *                              channel ID (NEW → inject as priority +
 *                              tv:newRelease).
 *   - OrderedPlaylistChannel : optional RSS (only if `rssChannelId`
 *                              configured, e.g. Popcorn) with a title
 *                              pattern + min-duration filter. No
 *                              priority injection, just a playlist-
 *                              reload notification (ordered channels
 *                              don't carry a priority queue concept).
 *   - FixedVideoChannel      : `pollRss()` returns [] — static
 *                              playlists never grow.
 */
async function pollChannel(channel) {
  const newVideos = await channel.pollRss();
  if (newVideos.length === 0) return;

  addNewVideos(channel.id, newVideos);
  await pubsub.publish(pubsub.CHANNELS.PLAYLIST_RELOAD, { channelId: channel.id });

  const emitter = getEmitter();

  // Normal channels inject each new video as a priority (plays next in
  // rotation). Ordered channels don't — their order is curated.
  if (channel.kind === 'normal') {
    for (const video of newVideos) {
      await pubsub.publish(pubsub.CHANNELS.PRIORITY_VIDEO, { channelId: channel.id, video });
    }
  }

  emitter.emit('tv:playlistUpdated', { channelId: channel.id });
  emitter.emit('tv:newRelease', { channelId: channel.id, count: newVideos.length });

  log.info(
    { channel: channel.id, kind: channel.kind, count: newVideos.length },
    'rss: new videos injected'
  );
}

async function pollOnce() {
  for (const channel of config.CHANNELS) {
    try {
      await pollChannel(channel);
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

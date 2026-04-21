/**
 * Bridge worker → web for internal state changes.
 *
 * The worker runs RSS poll + daily maintenance in a separate process.
 * When it mutates playlist state (writes new JSON to disk, or detects
 * a new priority video), it publishes to Redis on the `koala:*`
 * channels. This subscriber runs in the WEB process and:
 *
 *   - `koala:playlist-reload {channelId}`  → reload the JSON and emit
 *     a fresh tv:state to clients watching that channel
 *   - `koala:priority-video {channelId, video}`  → insert into the
 *     web's in-memory priority queue so the next getTvState returns it
 *
 * Socket.IO events destined for end clients (`tv:refreshed`,
 * `chat:cleared`, `maintenance:start/end`, …) travel directly via
 * `@socket.io/redis-emitter` and never pass through this bridge.
 */
const log = require('../services/logger');
const pubsub = require('../services/pubsub');
const { reloadFromDisk } = require('../services/playlist');
const { queuePriorityVideo } = require('../services/tv');
const { getTvState } = require('../services/tv');

let subClient = null;

function start(io) {
  subClient = pubsub.createSubscriber();

  subClient.on('message', (channel, raw) => {
    let msg;
    try { msg = JSON.parse(raw); } catch { return; }

    if (channel === pubsub.CHANNELS.PLAYLIST_RELOAD) {
      const { channelId } = msg;
      if (!channelId) return;
      const loaded = reloadFromDisk(channelId);
      if (!loaded) {
        log.warn({ channelId }, 'playlist-reload: file not found on disk');
        return;
      }
      // Tell every client currently on this channel to pick up the
      // fresh state on the next tick. We don't push tv:state here
      // because the 15 s sync broadcast will fan it out shortly — but
      // we do notify immediately so the client can decide.
      const state = getTvState(channelId);
      if (state && io) {
        io.to(`channel:${channelId}`).emit('tv:state', state);
      }
      log.debug({ channelId }, 'playlist reloaded from worker event');
      return;
    }

    if (channel === pubsub.CHANNELS.PRIORITY_VIDEO) {
      const { channelId, video } = msg;
      if (!channelId || !video) return;
      queuePriorityVideo(channelId, video);
      log.debug({ channelId, videoId: video.videoId }, 'priority video queued from worker');
      return;
    }
  });

  subClient.subscribe(
    pubsub.CHANNELS.PLAYLIST_RELOAD,
    pubsub.CHANNELS.PRIORITY_VIDEO,
    (err, count) => {
      if (err) {
        log.error({ err: err.message }, 'worker-bridge: subscribe failed');
        return;
      }
      log.info({ subscriptions: count }, 'worker-bridge: subscribed to worker pub/sub');
    }
  );
}

function stop() {
  if (subClient) {
    subClient.quit().catch(() => {});
    subClient = null;
  }
}

module.exports = { start, stop };

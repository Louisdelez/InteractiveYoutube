/**
 * Internal worker ↔ web pub/sub bus. Used for state-changing events
 * that can't cross processes via Socket.IO alone (e.g., "worker just
 * wrote a new playlist JSON — web, reload it from disk").
 *
 * Channel naming: `koala:<kind>`.
 *
 * Why not Socket.IO for this? Socket.IO events go to *clients*, not
 * to the peer server process. We want the web to update its in-memory
 * state before it next answers a tv:state request.
 */
const { redis } = require('./redis');
const IORedis = require('ioredis');
const config = require('../config');

const CHANNELS = {
  PLAYLIST_RELOAD: 'koala:playlist-reload',   // { channelId }
  PRIORITY_VIDEO: 'koala:priority-video',     // { channelId, video }
};

function publish(channel, payload) {
  return redis.publish(channel, JSON.stringify(payload || {}));
}

// Dedicated subscriber connection — ioredis subscribe mode blocks the
// client from issuing other commands, so we can't reuse the main one.
function createSubscriber() {
  return new IORedis(config.REDIS_URL);
}

module.exports = { CHANNELS, publish, createSubscriber };

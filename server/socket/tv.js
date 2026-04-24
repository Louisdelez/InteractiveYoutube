const { getTvState, enrichWithResolvedUrl } = require('../services/tv');
const config = require('../config');
const metrics = require('../services/metrics');
const log = require('../services/logger').child({ component: 'socket:tv' });

let syncInterval = null;

function registerTvHandlers(io, socket) {
  // NOTE: room join/leave + viewer count are handled in `socket/index.js`
  // (single source of truth, debounced). This module only handles
  // tv:state pushes + ping/pong + error reports.
  //
  // The initial `tv:state` is emitted from `socket/index.js` AFTER
  // `joinChannel()` so it carries the state of the channel the socket
  // was actually joined to — previously we hard-coded CHANNELS[0]
  // (Amixem) here, which clashed with the random `defaultChannel`
  // the connection handler uses for the room membership. The client
  // would land on Amixem, then 15 s later the first `tv:sync` for the
  // (random) room channel pulled it onto yet another chaîne — that's
  // the "random self-switch" users were seeing.
  const defaultChannel = config.CHANNELS[0].id;

  // RTT ping/pong
  socket.on('tv:ping', (clientTime) => {
    if (typeof clientTime !== 'number') return;
    socket.emit('tv:pong', { clientTime, serverTime: Date.now() });
  });

  // On switch, push the new channel's state. socket/index.js has
  // already (or will momentarily) updated `socket.currentChannel`
  // and the room membership.
  socket.on('tv:switchChannel', async (channelId) => {
    if (typeof channelId !== 'string') return;
    if (!config.CHANNELS.some((c) => c.id === channelId)) return;
    const state = getTvState(channelId);
    if (state) {
      socket.emit('tv:state', await enrichWithResolvedUrl(state));
    }
  });

  socket.on('tv:requestState', async () => {
    const state = getTvState(socket.currentChannel || defaultChannel);
    if (state) {
      socket.emit('tv:state', await enrichWithResolvedUrl(state));
    }
  });

  socket.on('tv:videoError', (data) => {
    // (kept for diagnostics — moved to debug to reduce log noise)
    if (data?.videoId) {
      // log via central logger if needed; left silent here
    }
  });
}

function startSyncBroadcast(io) {
  if (syncInterval) return;

  syncInterval = setInterval(() => {
    const start = Date.now();
    for (const channel of config.CHANNELS) {
      const state = getTvState(channel.id);
      if (state) {
        io.to(`channel:${channel.id}`).volatile.emit('tv:sync', state);
      }
    }
    metrics.syncBroadcastDuration.observe(Date.now() - start);
  }, config.DRIFT_CORRECTION_INTERVAL_MS);

  log.info({ intervalMs: config.DRIFT_CORRECTION_INTERVAL_MS }, 'tv:sync broadcast started');
}

function stopSyncBroadcast() {
  if (syncInterval) {
    clearInterval(syncInterval);
    syncInterval = null;
  }
}

module.exports = { registerTvHandlers, startSyncBroadcast, stopSyncBroadcast };

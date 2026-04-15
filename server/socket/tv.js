const { getTvState } = require('../services/tv');
const config = require('../config');
const metrics = require('../services/metrics');

let syncInterval = null;

function registerTvHandlers(io, socket) {
  // NOTE: room join/leave + viewer count are handled in `socket/index.js`
  // (single source of truth, debounced). This module only handles
  // tv:state pushes + ping/pong + error reports.
  const defaultChannel = config.CHANNELS[0].id;

  // Send initial state for default channel.
  const state = getTvState(defaultChannel);
  if (state) {
    socket.emit('tv:state', state);
  }

  // RTT ping/pong
  socket.on('tv:ping', (clientTime) => {
    if (typeof clientTime !== 'number') return;
    socket.emit('tv:pong', { clientTime, serverTime: Date.now() });
  });

  // On switch, push the new channel's state. socket/index.js has
  // already (or will momentarily) updated `socket.currentChannel`
  // and the room membership.
  socket.on('tv:switchChannel', (channelId) => {
    if (typeof channelId !== 'string') return;
    if (!config.CHANNELS.some((c) => c.id === channelId)) return;
    const state = getTvState(channelId);
    if (state) {
      socket.emit('tv:state', state);
    }
  });

  socket.on('tv:requestState', () => {
    const state = getTvState(socket.currentChannel || defaultChannel);
    if (state) {
      socket.emit('tv:state', state);
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

  console.log(`[TV] Sync broadcast started (every ${config.DRIFT_CORRECTION_INTERVAL_MS / 1000}s)`);
}

function stopSyncBroadcast() {
  if (syncInterval) {
    clearInterval(syncInterval);
    syncInterval = null;
  }
}

module.exports = { registerTvHandlers, startSyncBroadcast, stopSyncBroadcast };

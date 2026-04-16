const { Server } = require('socket.io');
const { createAdapter } = require('@socket.io/redis-adapter');
const jwt = require('jsonwebtoken');
const config = require('../config');
const { redisPub, redisSub, redis } = require('../services/redis');
const { registerTvHandlers, startSyncBroadcast, stopSyncBroadcast } = require('./tv');
const { registerChatHandlers, stopBatching } = require('./chat');
const { findUserById } = require('../db');
const log = require('../services/logger');
const metrics = require('../services/metrics');

let io = null;
let reconcileInterval = null;

// Per-channel viewer presence — a Redis Set of socket IDs.
// Replaces the old INCR/DECR counter that drifted permanently when
// a process crashed mid-handler or a switchChannel race lost a count.
// Authoritative count = SCARD; reconciled every 60 s against actual
// adapter sockets to drop stale entries from process crashes.
const viewerSetKey = (channelId) => `viewers:set:${channelId}`;
const ALL_CHANNEL_IDS = config.CHANNELS.map((c) => c.id);

// Per-IP connection limit shared across instances via Redis. Without
// this, the limit was process-local and trivially bypassed behind a
// load balancer.
const ipKey = (ip) => `iy:ip:${ip}`;
// Dev-friendly budget: 30 connexions par IP par 10 min. Les reloads
// rapides (F5, kill -9 du desktop) ne décrémentent pas toujours le
// compteur ; un TTL court garantit qu'on ne bloque pas sa propre
// session pendant 1 h après 10 restarts consécutifs.
const MAX_CONNECTIONS_PER_IP = 30;
const IP_TTL_SECS = 600;

// Per-socket throttle for tv:switchChannel — prevents Redis thrash
// when a user spams channel buttons.
const SWITCH_DEBOUNCE_MS = 400;

async function emitViewerCount(channelId) {
  const count = await redis.scard(viewerSetKey(channelId));
  io.to(`channel:${channelId}`).volatile.emit('viewers:count', { count });
  metrics.viewersGauge.set({ channel: channelId }, count);
  // Recompute and broadcast the global total so every client's topbar
  // ticker stays in sync. SCARD is O(1), and 48 channels = a single
  // pipelined Redis round-trip.
  await emitViewerTotal();
  return count;
}

async function emitViewerTotal() {
  if (!io) return 0;
  try {
    const pipe = redis.pipeline();
    for (const channelId of ALL_CHANNEL_IDS) {
      pipe.scard(viewerSetKey(channelId));
    }
    const results = await pipe.exec();
    const total = results.reduce((sum, [err, n]) => sum + (err ? 0 : (n || 0)), 0);
    io.volatile.emit('viewers:total', { total });
    return total;
  } catch (err) {
    log.warn({ err: err.message }, 'emitViewerTotal failed');
    return 0;
  }
}

async function joinChannel(socket, channelId) {
  await redis.sadd(viewerSetKey(channelId), socket.id);
  socket.join(`channel:${channelId}`);
  socket.currentChannel = channelId;
  return emitViewerCount(channelId);
}

async function leaveChannel(socket, channelId) {
  await redis.srem(viewerSetKey(channelId), socket.id);
  socket.leave(`channel:${channelId}`);
  return emitViewerCount(channelId);
}

/**
 * Periodic reconciliation: take the Redis SET of viewers for each
 * channel and remove any socket IDs that no longer exist on this
 * cluster (process crash, OOM, etc). Catches the leaks INCR/DECR
 * couldn't.
 */
async function reconcileViewers() {
  if (!io) return;
  try {
    const sockets = await io.fetchSockets(); // cluster-wide via Redis adapter
    const liveIds = new Set(sockets.map((s) => s.id));
    for (const channelId of ALL_CHANNEL_IDS) {
      const stored = await redis.smembers(viewerSetKey(channelId));
      const stale = stored.filter((id) => !liveIds.has(id));
      if (stale.length > 0) {
        await redis.srem(viewerSetKey(channelId), ...stale);
        log.info({ channel: channelId, dropped: stale.length }, 'reconciled stale viewers');
      }
      await emitViewerCount(channelId);
    }
  } catch (err) {
    log.error({ err: err.message }, 'viewer reconciliation failed');
  }
}

function setupSocketIO(httpServer) {
  io = new Server(httpServer, {
    cors: {
      origin: [config.CLIENT_ORIGIN, 'tauri://localhost', 'https://tauri.localhost'],
      credentials: true,
    },
    transports: ['websocket', 'polling'],
    maxHttpBufferSize: 16 * 1024,
    pingTimeout: 20000,
    pingInterval: 25000,
    connectTimeout: 10000,
  });

  io.adapter(createAdapter(redisPub, redisSub));
  log.info('redis adapter connected');

  // Auth + IP rate limit middleware. Both implemented via Redis so
  // they work across multiple server processes / hosts.
  io.use(async (socket, next) => {
    const ip = socket.handshake.headers['x-forwarded-for'] || socket.handshake.address;
    socket.clientIP = ip;
    try {
      const ipCount = await redis.incr(ipKey(ip));
      if (ipCount === 1) {
        await redis.expire(ipKey(ip), IP_TTL_SECS);
      }
      if (ipCount > MAX_CONNECTIONS_PER_IP) {
        await redis.decr(ipKey(ip));
        return next(new Error('Too many connections from this IP'));
      }
    } catch (err) {
      // Redis hiccup — fail open rather than block all auth.
      log.warn({ err: err.message }, 'ip-limit redis check failed');
    }

    const cookieHeader = socket.handshake.headers.cookie;
    if (cookieHeader) {
      const cookies = require('cookie').parse(cookieHeader);
      const token = cookies.token;
      if (token) {
        try {
          const decoded = jwt.verify(token, config.JWT_SECRET);
          socket.user = await findUserById(decoded.userId);
        } catch (err) {
          // Anonymous fallback
        }
      }
    }
    next();
  });

  io.on('connection', async (socket) => {
    // Pick a random channel per connection so every fresh client lands on
    // a different chaîne instead of always the first in the config list.
    const defaultChannel =
      config.CHANNELS[Math.floor(Math.random() * config.CHANNELS.length)].id;

    // Register chat first (it consumes socket.user). TV handler
    // registers its own switchChannel listener; the count update
    // below is wired via a single source of truth (joinChannel /
    // leaveChannel) called from this file — the tv.js handler only
    // mutates `socket.currentChannel` and emits state.
    registerTvHandlers(io, socket);
    registerChatHandlers(io, socket, socket.user);

    await joinChannel(socket, defaultChannel);
    socket.emit('viewers:count', {
      count: await redis.scard(viewerSetKey(defaultChannel)),
    });
    // Send the global total to the new client so the topbar ticker
    // populates immediately, before the first emitViewerTotal broadcast.
    try {
      const pipe = redis.pipeline();
      for (const cid of ALL_CHANNEL_IDS) pipe.scard(viewerSetKey(cid));
      const r = await pipe.exec();
      const total = r.reduce((s, [err, n]) => s + (err ? 0 : (n || 0)), 0);
      socket.emit('viewers:total', { total });
    } catch {}
    metrics.connectionsCounter.inc();
    log.info(
      {
        sid: socket.id,
        channel: defaultChannel,
        user: socket.user ? socket.user.username : 'anonymous',
      },
      'socket connected'
    );

    // Channel switch with debounce. We OWN the count update; the
    // tv.js handler only updates `socket.currentChannel`.
    let lastSwitchAt = 0;
    socket.on('tv:switchChannel', async (newChannelId) => {
      const now = Date.now();
      if (now - lastSwitchAt < SWITCH_DEBOUNCE_MS) {
        return; // dropped — user is mashing channel buttons
      }
      lastSwitchAt = now;
      if (typeof newChannelId !== 'string') return;
      if (!ALL_CHANNEL_IDS.includes(newChannelId)) return;
      const oldChannel = socket.currentChannel || defaultChannel;
      if (oldChannel === newChannelId) return;
      await leaveChannel(socket, oldChannel);
      await joinChannel(socket, newChannelId);
    });

    socket.on('disconnect', async () => {
      const channel = socket.currentChannel || defaultChannel;
      try {
        await redis.srem(viewerSetKey(channel), socket.id);
        await emitViewerCount(channel);
      } catch (err) {
        log.warn({ err: err.message, sid: socket.id }, 'disconnect cleanup failed');
      }
      try {
        await redis.decr(ipKey(socket.clientIP));
      } catch (_) {}
      log.info({ sid: socket.id }, 'socket disconnected');
    });
  });

  // Start the sync broadcast + reconciliation
  startSyncBroadcast(io);
  reconcileInterval = setInterval(reconcileViewers, 60_000);

  return io;
}

function getIO() {
  return io;
}

function shutdown() {
  stopBatching();
  stopSyncBroadcast();
  if (reconcileInterval) {
    clearInterval(reconcileInterval);
    reconcileInterval = null;
  }
  if (io) {
    io.close();
  }
}

module.exports = { setupSocketIO, getIO, shutdown };

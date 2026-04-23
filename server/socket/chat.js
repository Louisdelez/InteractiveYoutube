const { nanoid } = require('nanoid');
const config = require('../config');
const { redis } = require('../services/redis');
const log = require('../services/logger');
const metrics = require('../services/metrics');
const { generatePseudo, generateColor } = require('../services/pseudo');

/**
 * Strip control characters, zero-width spaces and bidirectional
 * override marks. Without this, an attacker can inject `\u202E` to
 * render text right-to-left (spoofing usernames, hiding code, etc).
 */
function sanitizeText(input) {
  // Drop C0/C1 controls, zero-width spaces (but NOT the Zero-Width
  // Joiner U+200D which is essential for combined emoji like
  // 👨‍💻 👩‍❤️‍👨 😶‍🌫️), BOM, bidi overrides.
  return input.replace(
    /[\u0000-\u001F\u007F-\u009F\u200B\u200C\u200E\u200F\u202A-\u202E\u2066-\u2069\uFEFF]/g,
    ''
  );
}

/**
 * Cap by Unicode codepoint count — `String.prototype.slice` operates
 * on UTF-16 code units, which corrupts surrogate pairs (emoji).
 */
function clampCodepoints(input, max) {
  const cps = Array.from(input);
  return cps.length > max ? cps.slice(0, max).join('') : input;
}

// HH:MM formatter — uses the server's TZ (process TZ, driven by the
// TZ env var if set). Single instance to avoid per-message allocation.
const timeFormatter = new Intl.DateTimeFormat('fr-FR', {
  hour: '2-digit',
  minute: '2-digit',
  timeZone: config.SERVER_TZ || undefined,
});
function formatServerTime(date) {
  return timeFormatter.format(date);
}

// Redis keys per channel
const chatHistoryKey = (channelId) => `chat:history:${channelId}`;
const RATE_KEY_PREFIX = 'chat:rate:';

// Message batching per channel
const pendingBatches = new Map(); // channelId -> []
let batchTimer = null;
let ioRef = null;

function flushAllBatches() {
  for (const [channelId, batch] of pendingBatches) {
    if (batch.length === 0) continue;
    ioRef.to(`channel:${channelId}`).volatile.emit('chat:batch', batch);
    pendingBatches.set(channelId, []);
  }
}

function startBatching(io) {
  ioRef = io;
  if (batchTimer) return;
  batchTimer = setInterval(flushAllBatches, config.CHAT_BATCH_INTERVAL_MS);
}

function stopBatching() {
  if (batchTimer) {
    clearInterval(batchTimer);
    batchTimer = null;
  }
  flushAllBatches();
}

// Rate limiting via Redis. Keyed by user.id (if logged in) or
// client IP (otherwise) — keying by socket.id was trivially bypassed
// by reconnecting in a loop.
async function isRateLimited(rateKey) {
  const key = RATE_KEY_PREFIX + rateKey;
  const now = Date.now();
  const windowStart = now - config.CHAT_RATE_WINDOW_MS;

  const pipe = redis.pipeline();
  pipe.zremrangebyscore(key, 0, windowStart);
  pipe.zcard(key);
  pipe.zadd(key, now, `${now}:${nanoid(4)}`);
  pipe.pexpire(key, config.CHAT_RATE_WINDOW_MS + 1000);

  const results = await pipe.exec();
  const count = results[1][1];
  return count >= config.CHAT_RATE_MAX_MESSAGES;
}

// Chat history in Redis per channel
async function pushToHistory(channelId, message) {
  const json = JSON.stringify(message);
  const pipe = redis.pipeline();
  pipe.rpush(chatHistoryKey(channelId), json);
  pipe.ltrim(chatHistoryKey(channelId), -config.CHAT_BUFFER_SIZE, -1);
  await pipe.exec();
}

async function getHistory(channelId) {
  const items = await redis.lrange(chatHistoryKey(channelId), 0, -1);
  return items.map((item) => JSON.parse(item));
}

// Anonymous names + colors
const anonymousNames = new Map();
const anonymousColors = new Map();

function registerChatHandlers(io, socket, user) {
  startBatching(io);

  // NOTE: initial chat:history emission was moved to socket/index.js
  // (after `joinChannel()`) so `socket.currentChannel` is set. The
  // old code here used CHANNELS[0] (Amixem) regardless of which room
  // the socket was actually joined to.

  // When channel changes, send new history. Validate channelId.
  socket.on('chat:channelChanged', (channelId) => {
    if (typeof channelId !== 'string') return;
    if (!config.CHANNELS.some((c) => c.id === channelId)) return;
    getHistory(channelId)
      .then((history) => socket.emit('chat:history', history))
      .catch((err) => log.error({ err: err.message }, 'failed to load chat history'));
  });

  // Anonymous name + color
  socket.on('chat:setAnonymousName', (payload) => {
    log.debug({ sid: socket.id, payload }, 'chat:setAnonymousName received');
    const name = payload && payload.name;
    const color = payload && payload.color;
    if (name && typeof name === 'string' && name.length <= 30) {
      anonymousNames.set(socket.id, name);
    }
    if (color && typeof color === 'string' && color.length <= 50) {
      anonymousColors.set(socket.id, color);
    }
  });

  // Handle message — goes to the socket's current channel
  socket.on('chat:message', async ({ text }) => {
    if (!text || typeof text !== 'string') return;

    // Sanitize first (strip control / RTL / zero-width chars), then
    // clamp by codepoint count (NOT UTF-16 unit, which would split
    // emoji surrogate pairs).
    const cleaned = sanitizeText(text).trim();
    const trimmed = clampCodepoints(cleaned, 500);
    if (trimmed.length === 0) return;

    // Rate-limit by user.id if logged in, else by client IP. Keying
    // by socket.id was bypassable: reconnect, fresh budget.
    const rateKey = (user && `u:${user.id}`) || `ip:${socket.clientIP || 'unknown'}`;
    try {
      if (await isRateLimited(rateKey)) {
        socket.emit('chat:error', { error: 'Trop de messages, attends un peu.' });
        return;
      }
    } catch (err) {
      log.warn({ err: err.message }, 'rate limit check failed');
    }

    let username, color, registered;
    if (user) {
      username = user.username;
      color = user.color || '#1E90FF';
      registered = true;
    } else {
      // Fallback: animal/fruit/legume FR pseudo, never the bare "Anonyme-".
      // The client (web + desktop) emits `chat:setAnonymousName` on connect,
      // but a chat:message can arrive first if the user is fast or if the
      // setAnonymousName packet got dropped during the namespace handshake.
      // In that case we generate one server-side and stash it for the
      // socket — same pseudo for the rest of the session.
      let stored = anonymousNames.get(socket.id);
      if (!stored) {
        stored = generatePseudo();
        anonymousNames.set(socket.id, stored);
      }
      username = stored;
      let storedColor = anonymousColors.get(socket.id);
      if (!storedColor) {
        storedColor = generateColor();
        anonymousColors.set(socket.id, storedColor);
      }
      color = storedColor;
      registered = false;
      log.debug(
        { sid: socket.id, used_stored: !!stored, username, color },
        'chat:message anonymous identity'
      );
    }

    const channelId = socket.currentChannel || defaultChannel;

    // Format the display time on the server so every client (no matter
    // their machine TZ) sees the same HH:MM. Server TZ is set via the
    // TZ env var (defaults to the host TZ).
    const now = new Date();
    const time = formatServerTime(now);

    const message = {
      id: nanoid(),
      text: trimmed,
      username,
      color,
      registered,
      timestamp: now.getTime(),
      time,
      channelId,
    };

    pushToHistory(channelId, message).catch((err) =>
      log.error({ err: err.message }, 'failed to push chat history')
    );

    if (!pendingBatches.has(channelId)) pendingBatches.set(channelId, []);
    pendingBatches.get(channelId).push(message);
    metrics.chatMessagesCounter.inc({ channel: channelId });
  });

  socket.on('disconnect', () => {
    anonymousNames.delete(socket.id);
    anonymousColors.delete(socket.id);
  });
}

/**
 * Wipe every channel's chat history in Redis and notify connected clients
 * so they clear their local view too. Called from the daily 3am cron.
 */
async function clearAllChatHistory(io) {
  let cursor = '0';
  const keys = [];
  do {
    const [next, batch] = await redis.scan(
      cursor, 'MATCH', 'chat:history:*', 'COUNT', 500
    );
    cursor = next;
    keys.push(...batch);
  } while (cursor !== '0');

  // Legacy orphan from pre-per-channel refactor: single `chat:history` list
  // that SCAN 'chat:history:*' never matches. Delete unconditionally —
  // no current code reads or writes it.
  keys.push('chat:history');

  if (keys.length > 0) await redis.del(...keys);
  if (io) io.emit('chat:cleared');
  return keys.length;
}

module.exports = { registerChatHandlers, stopBatching, clearAllChatHistory, getHistory };

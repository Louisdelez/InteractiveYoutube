/**
 * POST /api/logs — ingestion endpoint for web + desktop clients.
 *
 * Body is either a single event or { events: [...] } (batch).
 * Each event: { source, level, msg, ctx?, ts?, sessionId? }
 *
 * Events are re-logged via the server pino logger with source=web|desktop,
 * so they land in logs/server*.log alongside native server logs.
 *
 * No auth (diagnostic channel, must work for logged-out users) but
 * heavily rate-limited + size-capped to keep it from becoming a vector.
 */
const express = require('express');
const rateLimit = require('express-rate-limit');
const log = require('../services/logger');
const { t } = require('../i18n/fr');

const router = express.Router();

const VALID_LEVELS = new Set(['trace', 'debug', 'info', 'warn', 'error', 'fatal']);
const VALID_SOURCES = new Set(['web', 'desktop']);
const MAX_MSG_LEN = parseInt(process.env.LOG_MAX_MSG_LEN) || 4000;
const MAX_CTX_BYTES = parseInt(process.env.LOG_MAX_CTX_BYTES) || 8000;
const MAX_EVENTS_PER_BATCH = parseInt(process.env.LOG_MAX_EVENTS_PER_BATCH) || 50;

// 120 req/min/ip — a typical session flushes a batch every 5 s, so the
// cap only bites on real floods (runaway error loop, malicious client).
const limiter = rateLimit({
  windowMs: parseInt(process.env.LOG_RATE_WINDOW_MS) || 60 * 1000,
  max: parseInt(process.env.LOG_RATE_MAX) || 120,
  standardHeaders: 'draft-7',
  legacyHeaders: false,
  message: { error: t('logs.error.rate_limit') },
});

function sanitizeEvent(raw, reqIp) {
  if (!raw || typeof raw !== 'object') return null;

  const source = VALID_SOURCES.has(raw.source) ? raw.source : null;
  const level = VALID_LEVELS.has(raw.level) ? raw.level : 'info';
  if (!source) return null;

  const msg = typeof raw.msg === 'string' ? raw.msg.slice(0, MAX_MSG_LEN) : '';
  if (!msg) return null;

  let ctx;
  if (raw.ctx && typeof raw.ctx === 'object') {
    try {
      const s = JSON.stringify(raw.ctx);
      if (s.length <= MAX_CTX_BYTES) ctx = raw.ctx;
      else ctx = { _truncated: true };
    } catch {
      ctx = undefined;
    }
  }

  const ts = typeof raw.ts === 'number' && Number.isFinite(raw.ts) ? raw.ts : Date.now();
  const sessionId = typeof raw.sessionId === 'string' ? raw.sessionId.slice(0, 64) : undefined;

  return {
    source,
    level,
    msg,
    ctx,
    ts,
    sessionId,
    ip: reqIp,
  };
}

router.post('/', limiter, express.json({ limit: '256kb' }), (req, res) => {
  const body = req.body || {};
  const rawEvents = Array.isArray(body.events) ? body.events : [body];

  if (rawEvents.length === 0 || rawEvents.length > MAX_EVENTS_PER_BATCH) {
    return res.status(400).json({ error: t('logs.error.invalid_batch') });
  }

  let accepted = 0;
  for (const raw of rawEvents) {
    const ev = sanitizeEvent(raw, req.ip);
    if (!ev) continue;

    const fields = {
      source: ev.source,
      clientTs: ev.ts,
      sessionId: ev.sessionId,
      ip: ev.ip,
      ...(ev.ctx || {}),
    };
    log[ev.level](fields, `[${ev.source}] ${ev.msg}`);
    accepted++;
  }

  res.status(202).json({ accepted, received: rawEvents.length });
});

module.exports = router;

/**
 * Koala TV web logger.
 *
 * - mirrors logs to the console (unchanged dev workflow)
 * - buffers events in-memory (ring of 200) so a bug report can dump
 *   recent history via `window.__koalaLogBuffer`
 * - batch-flushes to POST /api/logs every 5 s (or when 20 events queue,
 *   or on page hide via sendBeacon)
 * - errors flush immediately (best-effort, still queued if offline)
 *
 * Import from everywhere:
 *   import { log } from '@/services/logger';   // or relative path
 *   log.info('connected', { channel });
 */
import { isTauri } from './platform';

const LOG_ENDPOINT = (isTauri()
  ? (localStorage.getItem('iyt-server-url') || 'http://localhost:4500')
  : ''
) + '/api/logs';

const LEVELS = ['debug', 'info', 'warn', 'error'];
const FLUSH_INTERVAL_MS = 5000;
const BATCH_THRESHOLD = 20;
const RING_CAP = 200;

const sessionId = (() => {
  try {
    let s = sessionStorage.getItem('koala-log-sid');
    if (!s) {
      s = Math.random().toString(36).slice(2) + Date.now().toString(36);
      sessionStorage.setItem('koala-log-sid', s);
    }
    return s;
  } catch {
    return Math.random().toString(36).slice(2);
  }
})();

const queue = [];
const ring = [];
let flushTimer = null;

function push(level, msg, ctx) {
  const event = {
    source: 'web',
    level,
    msg: String(msg ?? ''),
    ctx: ctx && typeof ctx === 'object' ? ctx : undefined,
    ts: Date.now(),
    sessionId,
  };

  ring.push(event);
  if (ring.length > RING_CAP) ring.shift();

  // Mirror to browser console so existing dev habits still work.
  const consoleFn = console[level === 'debug' ? 'log' : level] || console.log;
  if (ctx) consoleFn(`[koala] ${msg}`, ctx);
  else consoleFn(`[koala] ${msg}`);

  queue.push(event);
  if (level === 'error' || queue.length >= BATCH_THRESHOLD) flush();
  else scheduleFlush();
}

function scheduleFlush() {
  if (flushTimer) return;
  flushTimer = setTimeout(() => {
    flushTimer = null;
    flush();
  }, FLUSH_INTERVAL_MS);
}

function flush() {
  if (queue.length === 0) return;
  if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }

  const events = queue.splice(0, queue.length);
  const body = JSON.stringify({ events });

  // fetch with keepalive so the request survives a navigation.
  try {
    fetch(LOG_ENDPOINT, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body,
      keepalive: true,
      credentials: 'include',
    }).catch(() => {
      // Swallow — logger must never throw. Failed batches are lost by
      // design; the ring buffer still holds recent events locally.
    });
  } catch {
    /* noop */
  }
}

function flushOnHide() {
  if (queue.length === 0) return;
  const events = queue.splice(0, queue.length);
  const body = JSON.stringify({ events });
  try {
    if (navigator.sendBeacon) {
      const blob = new Blob([body], { type: 'application/json' });
      navigator.sendBeacon(LOG_ENDPOINT, blob);
    } else {
      // fall back to keepalive fetch
      fetch(LOG_ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body,
        keepalive: true,
      }).catch(() => {});
    }
  } catch { /* noop */ }
}

export const log = {
  debug: (msg, ctx) => push('debug', msg, ctx),
  info: (msg, ctx) => push('info', msg, ctx),
  warn: (msg, ctx) => push('warn', msg, ctx),
  error: (msg, ctx) => push('error', msg, ctx),
  flush,
  sessionId,
};

/**
 * Call once at app boot. Installs:
 *  - window.onerror
 *  - unhandledrejection listener
 *  - pagehide flush
 *  - window.__koalaLogBuffer accessor for manual inspection
 */
export function installGlobalLogHandlers() {
  window.addEventListener('error', (e) => {
    log.error(e.message || 'window error', {
      filename: e.filename,
      lineno: e.lineno,
      colno: e.colno,
      stack: e.error && e.error.stack,
    });
  });

  window.addEventListener('unhandledrejection', (e) => {
    const reason = e.reason;
    log.error('unhandled promise rejection', {
      reason: reason && reason.message ? reason.message : String(reason),
      stack: reason && reason.stack,
    });
  });

  window.addEventListener('pagehide', flushOnHide);
  window.addEventListener('beforeunload', flushOnHide);

  window.__koalaLogBuffer = () => ring.slice();
  window.__koalaFlush = flush;
}

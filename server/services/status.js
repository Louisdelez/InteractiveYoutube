/**
 * Status page backend.
 *
 * Two concerns:
 *   1. Real-time health probes — parallel checks of each dependency
 *      (postgres, redis, youtube quota, cron freshness, yt-dlp, disk,
 *      loki). Returned synchronously by GET /api/status.
 *   2. Persistence — every 60 s a `runCollector` tick inserts one row
 *      per component into `status_checks`. GET /api/status/history
 *      aggregates that table into a daily uptime percentage for the
 *      last N days (the coloured strip on the page).
 *
 * Component status is tri-state:
 *   operational  — green  (ok + latency under soft threshold)
 *   degraded     — yellow (ok but slow, or non-critical error)
 *   down         — red    (check failed hard)
 */

const fs = require('fs');
const path = require('path');
const { pool } = require('../db');
const { redis } = require('./redis');
const config = require('../config');
const log = require('./logger');

// Status-collector knobs — all overridable via env so ops can retune
// without a redeploy. Defaults match the dev-box baseline; tighten for
// production.
const COLLECTOR_INTERVAL_MS = parseInt(process.env.STATUS_COLLECTOR_INTERVAL_MS) || 60 * 1000;
const YOUTUBE_PROBE_URL =
  process.env.STATUS_YOUTUBE_PROBE_URL ||
  'https://www.googleapis.com/youtube/v3/videos?part=id&id=dQw4w9WgXcQ&key=';
const LOKI_HEALTH_URL = process.env.LOKI_URL || 'http://localhost:3100/ready';
const DISK_PATH = process.env.STATUS_DISK_PATH || '/';

// Thresholds (ms) above which "operational" becomes "degraded".
// Each lane is individually overridable — e.g. STATUS_LATENCY_REDIS_MS=80.
const LATENCY_SOFT_MS = {
  postgres: parseInt(process.env.STATUS_LATENCY_POSTGRES_MS) || 150,
  redis: parseInt(process.env.STATUS_LATENCY_REDIS_MS) || 50,
  youtube: parseInt(process.env.STATUS_LATENCY_YOUTUBE_MS) || 1500,
  loki: parseInt(process.env.STATUS_LATENCY_LOKI_MS) || 500,
};

// Definition shared with the UI.
const COMPONENTS = [
  { id: 'server',    name: 'API server',       critical: true },
  { id: 'postgres',  name: 'Postgres',         critical: true },
  { id: 'redis',     name: 'Redis',            critical: true },
  { id: 'youtube',   name: 'YouTube API',      critical: true },
  { id: 'cron',      name: 'Daily maintenance', critical: false },
  { id: 'ytdlp',     name: 'yt-dlp binary',    critical: false },
  { id: 'loki',      name: 'Log pipeline',     critical: false },
  { id: 'disk',      name: 'Disk space',       critical: false },
];

async function initStatusSchema() {
  await pool.query(`
    CREATE TABLE IF NOT EXISTS status_checks (
      id BIGSERIAL PRIMARY KEY,
      ts TIMESTAMPTZ NOT NULL DEFAULT NOW(),
      component TEXT NOT NULL,
      status TEXT NOT NULL,
      latency_ms INTEGER,
      message TEXT
    )
  `);
  await pool.query(`
    CREATE INDEX IF NOT EXISTS idx_status_checks_comp_ts
      ON status_checks (component, ts DESC)
  `);
  await pool.query(`
    CREATE TABLE IF NOT EXISTS status_incidents (
      id SERIAL PRIMARY KEY,
      title TEXT NOT NULL,
      body TEXT,
      severity TEXT NOT NULL,
      started_at TIMESTAMPTZ NOT NULL,
      resolved_at TIMESTAMPTZ,
      components TEXT[] NOT NULL DEFAULT '{}',
      created_at TIMESTAMPTZ DEFAULT NOW()
    )
  `);
  await pool.query(`
    CREATE INDEX IF NOT EXISTS idx_status_incidents_started
      ON status_incidents (started_at DESC)
  `);
  log.info('status schema ready');
}

// ── Individual checks ───────────────────────────────────────────────

function withLatency(label, fn) {
  return async () => {
    const t0 = Date.now();
    try {
      const r = await fn();
      const latency = Date.now() - t0;
      const soft = LATENCY_SOFT_MS[label];
      let status = r.status || 'operational';
      if (status === 'operational' && soft && latency > soft) status = 'degraded';
      return { status, latency, message: r.message };
    } catch (err) {
      return {
        status: 'down',
        latency: Date.now() - t0,
        message: err.message || String(err),
      };
    }
  };
}

const checkServer = withLatency('server', async () => {
  return { status: 'operational', message: `uptime ${Math.round(process.uptime())}s` };
});

const checkPostgres = withLatency('postgres', async () => {
  const r = await pool.query('SELECT 1 AS ok');
  if (r.rows[0].ok !== 1) return { status: 'degraded', message: 'unexpected result' };
  return { status: 'operational' };
});

const checkRedis = withLatency('redis', async () => {
  const pong = await redis.ping();
  if (pong !== 'PONG') return { status: 'degraded', message: `ping returned ${pong}` };
  return { status: 'operational' };
});

const checkYouTube = withLatency('youtube', async () => {
  if (!config.YOUTUBE_API_KEY) {
    return { status: 'degraded', message: 'no API key configured' };
  }
  const controller = new AbortController();
  const timeout = setTimeout(
    () => controller.abort(),
    parseInt(process.env.STATUS_YOUTUBE_TIMEOUT_MS) || 4000,
  );
  try {
    const res = await fetch(YOUTUBE_PROBE_URL + config.YOUTUBE_API_KEY, {
      signal: controller.signal,
    });
    if (res.status === 403) {
      return { status: 'degraded', message: 'quota exceeded or key invalid' };
    }
    if (!res.ok) {
      return { status: 'down', message: `HTTP ${res.status}` };
    }
    return { status: 'operational' };
  } finally {
    clearTimeout(timeout);
  }
});

const CRON_LOG_PATH =
  process.env.CRON_LOG_PATH ||
  path.resolve(__dirname, '..', '..', 'logs', 'koala-cron.log');
const checkCron = withLatency('cron', async () => {
  try {
    const stat = fs.statSync(CRON_LOG_PATH);
    const ageHours = (Date.now() - stat.mtimeMs) / 3600_000;
    const downHours = parseFloat(process.env.STATUS_CRON_DOWN_HOURS) || 26;
    const degradedHours = parseFloat(process.env.STATUS_CRON_DEGRADED_HOURS) || 25;
    if (ageHours > downHours) {
      return {
        status: 'down',
        message: `last update ${ageHours.toFixed(1)}h ago — next cron missed`,
      };
    }
    if (ageHours > degradedHours) {
      return { status: 'degraded', message: `last update ${ageHours.toFixed(1)}h ago` };
    }
    return { status: 'operational', message: `last run ${ageHours.toFixed(1)}h ago` };
  } catch (err) {
    if (err.code === 'ENOENT') {
      // First-run before any cron has fired — declare operational (not
      // down) so a fresh install doesn't look broken.
      return { status: 'operational', message: 'no cron history yet' };
    }
    throw err;
  }
});

const checkYtdlp = withLatency('ytdlp', async () => {
  const binPath =
    process.env.YTDLP_BIN_PATH ||
    path.resolve(process.env.YTDLP_BIN_DIR || path.resolve(__dirname, '..', '..', 'bin'), 'yt-dlp');
  if (!fs.existsSync(binPath)) {
    return { status: 'degraded', message: 'binary not found, updater running?' };
  }
  const stat = fs.statSync(binPath);
  const ageDays = (Date.now() - stat.mtimeMs) / 86_400_000;
  const ytdlpDegradedDays = parseFloat(process.env.STATUS_YTDLP_DEGRADED_DAYS) || 14;
  if (ageDays > ytdlpDegradedDays) {
    return { status: 'degraded', message: `binary ${ageDays.toFixed(0)}d old` };
  }
  return { status: 'operational', message: `updated ${ageDays.toFixed(1)}d ago` };
});

const checkLoki = withLatency('loki', async () => {
  const controller = new AbortController();
  const timeout = setTimeout(
    () => controller.abort(),
    parseInt(process.env.STATUS_LOKI_TIMEOUT_MS) || 2000,
  );
  try {
    const res = await fetch(LOKI_HEALTH_URL, { signal: controller.signal });
    if (!res.ok) return { status: 'down', message: `HTTP ${res.status}` };
    return { status: 'operational' };
  } catch (err) {
    // Loki is optional — down means "page can't enrich with logs" but
    // doesn't affect TV playback. Keep it as degraded, not down.
    return { status: 'degraded', message: 'unreachable — stack not up?' };
  } finally {
    clearTimeout(timeout);
  }
});

const checkDisk = withLatency('disk', async () => {
  const { statfs } = fs.promises;
  if (!statfs) {
    return { status: 'operational', message: 'statfs unavailable' };
  }
  const s = await statfs(DISK_PATH);
  const totalBytes = s.blocks * s.bsize;
  const freeBytes = s.bavail * s.bsize;
  const pctFree = (freeBytes / totalBytes) * 100;
  const gbFree = freeBytes / 1_073_741_824;
  const msg = `${gbFree.toFixed(0)} GB free (${pctFree.toFixed(0)}%)`;
  const diskDownPct = parseFloat(process.env.STATUS_DISK_DOWN_PCT) || 5;
  const diskDegradedPct = parseFloat(process.env.STATUS_DISK_DEGRADED_PCT) || 15;
  if (pctFree < diskDownPct) return { status: 'down', message: msg };
  if (pctFree < diskDegradedPct) return { status: 'degraded', message: msg };
  return { status: 'operational', message: msg };
});

const CHECKS = {
  server: checkServer,
  postgres: checkPostgres,
  redis: checkRedis,
  youtube: checkYouTube,
  cron: checkCron,
  ytdlp: checkYtdlp,
  loki: checkLoki,
  disk: checkDisk,
};

// ── Live probe + global summary ─────────────────────────────────────

async function checkAll() {
  const ids = Object.keys(CHECKS);
  const results = await Promise.all(
    ids.map(async (id) => {
      const r = await CHECKS[id]();
      const def = COMPONENTS.find((c) => c.id === id);
      return {
        id,
        name: def.name,
        critical: def.critical,
        status: r.status,
        latencyMs: r.latency,
        message: r.message,
      };
    })
  );
  const summary = rollup(results);
  return { components: results, summary, ts: new Date().toISOString() };
}

function rollup(components) {
  const criticalDown = components.some((c) => c.critical && c.status === 'down');
  const anyDown = components.some((c) => c.status === 'down');
  const anyDegraded = components.some((c) => c.status === 'degraded');
  let overall = 'operational';
  if (criticalDown) overall = 'major_outage';
  else if (anyDown) overall = 'partial_outage';
  else if (anyDegraded) overall = 'degraded';
  return { overall };
}

// ── Persistence / history ───────────────────────────────────────────

async function recordSnapshot(snapshot) {
  const rows = snapshot.components.map((c) => [c.id, c.status, c.latencyMs ?? null, c.message || null]);
  if (rows.length === 0) return;
  // Build a multi-row INSERT — 4 params per row.
  const placeholders = rows.map((_, i) => {
    const o = i * 4;
    return `($${o + 1}, $${o + 2}, $${o + 3}, $${o + 4})`;
  }).join(', ');
  const params = rows.flat();
  await pool.query(
    `INSERT INTO status_checks (component, status, latency_ms, message) VALUES ${placeholders}`,
    params
  );
}

async function pruneOldChecks() {
  // Keep 95 days (a bit more than the 90-day strip on the page).
  await pool.query(`DELETE FROM status_checks WHERE ts < NOW() - INTERVAL '95 days'`);
}

let collectorTimer = null;
function startCollector() {
  if (collectorTimer) return;
  const tick = async () => {
    try {
      const snap = await checkAll();
      await recordSnapshot(snap);
    } catch (err) {
      log.error({ err: err.message }, 'status collector tick failed');
    }
  };
  tick(); // immediate, so the page has data on first open
  collectorTimer = setInterval(tick, COLLECTOR_INTERVAL_MS);

  // Prune daily.
  setInterval(() => {
    pruneOldChecks().catch((err) =>
      log.warn({ err: err.message }, 'status prune failed')
    );
  }, parseInt(process.env.STATUS_PRUNE_INTERVAL_MS) || 24 * 3600 * 1000);
}

function stopCollector() {
  if (collectorTimer) {
    clearInterval(collectorTimer);
    collectorTimer = null;
  }
}

/**
 * Daily rollup per component for the last `days` days. Status of a
 * day = worst status observed that day (down > degraded > operational).
 * Days with zero checks are returned as "unknown" so the UI can grey
 * them out.
 */
async function historyByDay(days = 90) {
  // Anchor the day bucket in UTC on both sides so there's no drift
  // between JS (local time) and SQL (session tz — usually UTC).
  const sql = `
    WITH by_day AS (
      SELECT
        component,
        (ts AT TIME ZONE 'UTC')::date AS day,
        MIN(
          CASE status
            WHEN 'down'       THEN 0
            WHEN 'degraded'   THEN 1
            WHEN 'operational' THEN 2
            ELSE 2
          END
        ) AS worst
      FROM status_checks
      WHERE ts >= NOW() - ($1::int || ' days')::interval
      GROUP BY component, day
    )
    SELECT component, day,
      CASE worst WHEN 0 THEN 'down' WHEN 1 THEN 'degraded' ELSE 'operational' END AS status
    FROM by_day
    ORDER BY component, day
  `;
  const r = await pool.query(sql, [days]);

  const now = new Date();
  const todayUTC = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const allDays = [];
  for (let i = days - 1; i >= 0; i--) {
    const d = new Date(todayUTC);
    d.setUTCDate(d.getUTCDate() - i);
    allDays.push(d.toISOString().slice(0, 10));
  }

  const grouped = {};
  for (const c of COMPONENTS) {
    grouped[c.id] = new Map(allDays.map((d) => [d, 'unknown']));
  }
  for (const row of r.rows) {
    const key = row.day.toISOString().slice(0, 10);
    if (grouped[row.component]?.has(key)) {
      grouped[row.component].set(key, row.status);
    }
  }

  const out = {};
  for (const id of Object.keys(grouped)) {
    out[id] = allDays.map((day) => ({ day, status: grouped[id].get(day) }));
  }
  return out;
}

async function listIncidents({ limit = 20 } = {}) {
  const r = await pool.query(
    `SELECT id, title, body, severity, started_at, resolved_at, components
     FROM status_incidents
     ORDER BY started_at DESC
     LIMIT $1`,
    [limit]
  );
  return r.rows;
}

module.exports = {
  COMPONENTS,
  initStatusSchema,
  checkAll,
  startCollector,
  stopCollector,
  historyByDay,
  listIncidents,
};

/**
 * Daily 3 am maintenance — runs in the worker process, scheduled via
 * BullMQ (no in-process node-cron). Retries on failure with exponential
 * backoff and resumes from a Redis checkpoint so a mid-job crash
 * doesn't re-run already-completed steps (important: step 1 yt-dlp
 * update is expensive, step 4 chat clear is irreversible).
 *
 *  Pipeline, each gated by a checkpoint key under `maint:ckpt:<jobId>`:
 *    1. yt-dlp self-update
 *    2. Refresh playlists (1/7 of channels, per-channel 60 s timeout)
 *    3. Redis cleanup (stale viewers, expired rate-limit keys)
 *    4. Wipe chat history (Redis SCAN + DEL)
 *    5. Verify empty (SCAN again, force-delete if any leftover)
 *    6. Emit maintenance:end to clients
 *
 * Replaces the old `dailyMaintenance()` in server/cron/refresh.js
 * which ran in-process and was killed by nodemon's watch on
 * server/data/*.json mid-pipeline.
 */
const { Queue, Worker } = require('bullmq');
const config = require('../config');
const log = require('../services/logger');
const metrics = require('../services/metrics');
const { redis } = require('../services/redis');
const { refreshPlaylist } = require('../services/playlist');
const { clearAllChatHistory } = require('../socket/chat');
const ytdlpUpdater = require('../services/ytdlp-updater');
const { createConnection } = require('./bullmq-connection');
const { getEmitter } = require('./io-emitter');
const pubsub = require('../services/pubsub');

const QUEUE_NAME = 'koala-maintenance';
const JOB_DAILY = 'daily-maintenance';
const JOB_WARNING = 'maintenance-warning';
const SCHED_DAILY = 'koala-daily-3am';
const SCHED_WARNING = 'koala-daily-2h55-warning';

const CKPT_TTL_SECS = parseInt(process.env.MAINT_CKPT_TTL_SECS) || 6 * 3600;
const REFRESH_TIMEOUT_MS = parseInt(process.env.MAINT_REFRESH_TIMEOUT_MS) || 60_000;

// Server-side source of truth for the maintenance banner. A client
// that reconnects mid-maintenance (or after missing `maintenance:end`
// because its websocket blipped at 3 am) can no longer get its banner
// wedged — `server/socket/index.js` replays the current value on every
// new connection. TTLs auto-clear stale state if anything crashes.
const STATE_KEY = 'koala:maint:state';
const STATE_WARNING_TTL_SECS = parseInt(process.env.MAINT_STATE_WARNING_TTL_SECS) || 15 * 60;
const STATE_RUNNING_TTL_SECS = parseInt(process.env.MAINT_STATE_RUNNING_TTL_SECS) || 15 * 60;

const withTimeout = (p, label) =>
  Promise.race([
    p,
    new Promise((_, reject) =>
      setTimeout(() => reject(new Error(`timeout >${REFRESH_TIMEOUT_MS / 1000}s`)), REFRESH_TIMEOUT_MS)
    ),
  ]).catch((err) => { throw new Error(`${label}: ${err.message}`); });

async function runDailyMaintenance(job) {
  const jobId = String(job.id);
  const ckptKey = `maint:ckpt:${jobId}`;
  const emitter = getEmitter();
  const start = Date.now();

  log.info({ jobId, attempt: job.attemptsMade + 1 }, 'maintenance: start');
  await redis.set(STATE_KEY, 'running', 'EX', STATE_RUNNING_TTL_SECS);
  emitter.emit('maintenance:start');

  try {
    const done = await redis.hgetall(ckptKey) || {};

    // ── Step 1: yt-dlp ─────────────────────────────────────────
    if (!done.ytdlp) {
      log.info('[1/5] yt-dlp self-update');
      try {
        await ytdlpUpdater.selfUpdate();
      } catch (err) {
        log.warn({ err: err.message }, '[1/5] yt-dlp update failed — continuing');
      }
      await redis.hset(ckptKey, 'ytdlp', '1');
      await redis.expire(ckptKey, CKPT_TTL_SECS);
    }

    // ── Step 2: Refresh playlists (1/7 bucket) ─────────────────
    if (!done.refresh) {
      const day = new Date().getDay();
      const bucket = config.CHANNELS.filter((_, i) => i % 7 === day);
      log.info({ day, count: bucket.length, total: config.CHANNELS.length }, '[2/5] refresh bucket');
      let ok = 0, fail = 0;
      for (const channel of bucket) {
        try {
          await withTimeout(refreshPlaylist(channel.id), channel.id);
          // Tell the web to reload this playlist from disk now that
          // the worker has written the new JSON.
          await pubsub.publish(pubsub.CHANNELS.PLAYLIST_RELOAD, { channelId: channel.id });
          emitter.to(`channel:${channel.id}`).emit('tv:refreshed');
          emitter.emit('tv:playlistUpdated', { channelId: channel.id });
          ok++;
        } catch (err) {
          log.warn({ channelId: channel.id, err: err.message }, '[2/5] refresh FAILED (skipped)');
          fail++;
        }
      }
      log.info({ ok, fail }, '[2/5] refresh done');
      await redis.hset(ckptKey, 'refresh', '1');
      await redis.expire(ckptKey, CKPT_TTL_SECS);
    }

    // ── Step 3: Redis cleanup ──────────────────────────────────
    if (!done.redisCleanup) {
      log.info('[3/5] redis cleanup');
      let cleanedRateKeys = 0;
      try {
        let cursor = '0';
        do {
          const [next, batch] = await redis.scan(cursor, 'MATCH', 'chat:rate:*', 'COUNT', 500);
          cursor = next;
          for (const key of batch) {
            const ttl = await redis.pttl(key);
            if (ttl <= 0) {
              await redis.del(key);
              cleanedRateKeys++;
            }
          }
        } while (cursor !== '0');
      } catch (err) {
        log.warn({ err: err.message }, '[3/5] rate-key cleanup failed (continuing)');
      }
      log.info({ cleanedRateKeys }, '[3/5] redis cleanup done');
      await redis.hset(ckptKey, 'redisCleanup', '1');
      await redis.expire(ckptKey, CKPT_TTL_SECS);
    }

    // ── Step 4: Clear chat history ─────────────────────────────
    if (!done.chat) {
      log.info('[4/5] chat history clear');
      const cleared = await clearAllChatHistory(emitter);
      log.info({ cleared }, '[4/5] chat cleared');
      await redis.hset(ckptKey, 'chat', '1');
      await redis.expire(ckptKey, CKPT_TTL_SECS);
    }

    // ── Step 5: Verify empty ───────────────────────────────────
    if (!done.verify) {
      let remaining = [];
      let cursor = '0';
      do {
        const [next, batch] = await redis.scan(cursor, 'MATCH', 'chat:history:*', 'COUNT', 500);
        cursor = next;
        remaining.push(...batch);
      } while (cursor !== '0');
      if (await redis.exists('chat:history')) remaining.push('chat:history');
      if (remaining.length > 0) {
        log.warn({ leftovers: remaining.length }, '[5/5] force-deleting leftover chat keys');
        await redis.del(...remaining);
      }
      log.info('[5/5] verified empty');
      await redis.hset(ckptKey, 'verify', '1');
    }

    // Final: notify clients, clean checkpoint, metrics, optional HC ping
    await redis.del(STATE_KEY);
    emitter.emit('maintenance:end');
    await redis.del(ckptKey);

    const durSec = +((Date.now() - start) / 1000).toFixed(1);
    // Write metrics to Redis — web's /metrics reads these keys at
    // scrape time (the worker process can't share a prom-client
    // registry with the web, so we round-trip through Redis).
    await redis.mset(
      metrics.MAINT_LAST_SUCCESS_KEY, Math.floor(Date.now() / 1000),
      metrics.MAINT_LAST_DURATION_KEY, durSec
    );

    log.info({ jobId, durSec }, 'maintenance: complete');

    if (process.env.HEALTHCHECKS_URL) {
      fetch(process.env.HEALTHCHECKS_URL, { method: 'GET' })
        .catch((err) => log.warn({ err: err.message }, 'healthchecks ping failed'));
    }

    return { ok: true, durSec };
  } catch (err) {
    await redis.set(metrics.MAINT_LAST_FAILURE_KEY, Math.floor(Date.now() / 1000)).catch(() => {});
    log.error({ jobId, err: err.message, stack: err.stack }, 'maintenance: FAILED');
    if (process.env.HEALTHCHECKS_URL) {
      fetch(`${process.env.HEALTHCHECKS_URL}/fail`, { method: 'POST', body: err.message })
        .catch(() => {});
    }
    throw err; // BullMQ retries based on job opts
  }
}

async function runWarning(job) {
  log.info('maintenance warning (T-5 min)');
  await redis.set(STATE_KEY, 'warning', 'EX', STATE_WARNING_TTL_SECS);
  getEmitter().emit('maintenance:warning', { minutes: 5 });
  return { warned: true };
}

/**
 * Register the repeatable schedulers and start a Worker that consumes
 * the queue. Call once from server/workers/index.js on boot.
 */
async function start() {
  const queue = new Queue(QUEUE_NAME, { connection: createConnection() });

  // upsertJobScheduler is idempotent — safe to call on every boot.
  // The scheduler name is the primary key; repeated calls update the
  // cron pattern / opts in place without creating duplicates.
  await queue.upsertJobScheduler(
    SCHED_DAILY,
    { pattern: config.DAILY_REFRESH_CRON, tz: config.SERVER_TZ },
    {
      name: JOB_DAILY,
      opts: {
        attempts: parseInt(process.env.MAINT_JOB_ATTEMPTS) || 3,
        backoff: {
          type: 'exponential',
          delay: parseInt(process.env.MAINT_JOB_BACKOFF_MS) || 60_000,
        },
        removeOnComplete: { age: 7 * 24 * 3600, count: 50 },
        removeOnFail: { age: 30 * 24 * 3600, count: 200 },
      },
    }
  );

  await queue.upsertJobScheduler(
    SCHED_WARNING,
    {
      pattern: process.env.DAILY_WARNING_CRON || '55 2 * * *',
      tz: config.SERVER_TZ,
    },
    {
      name: JOB_WARNING,
      opts: { removeOnComplete: true, removeOnFail: { count: 10 } },
    }
  );

  const worker = new Worker(
    QUEUE_NAME,
    async (job) => {
      if (job.name === JOB_DAILY) return runDailyMaintenance(job);
      if (job.name === JOB_WARNING) return runWarning(job);
      throw new Error(`unknown job name: ${job.name}`);
    },
    {
      connection: createConnection(),
      concurrency: 1, // one maintenance at a time, period
      lockDuration: parseInt(process.env.MAINT_LOCK_DURATION_MS) || 10 * 60 * 1000,
    }
  );

  worker.on('completed', (job, result) => {
    log.info({ jobId: job.id, name: job.name, result }, 'bull job completed');
  });
  worker.on('failed', (job, err) => {
    log.error(
      { jobId: job?.id, name: job?.name, attempt: job?.attemptsMade, err: err.message },
      'bull job failed'
    );
  });

  log.info({ queue: QUEUE_NAME, tz: config.SERVER_TZ }, 'maintenance scheduler armed');
  return { queue, worker };
}

/**
 * Enqueue a one-off maintenance run (used by /api/admin/maintenance-trigger).
 * Creates its own short-lived Queue connection.
 */
async function triggerNow({ delay = 0 } = {}) {
  const queue = new Queue(QUEUE_NAME, { connection: createConnection() });
  const job = await queue.add(
    JOB_DAILY,
    { manual: true, startedAt: new Date().toISOString() },
    {
      attempts: 1,
      delay,
      removeOnComplete: { age: 3600, count: 10 },
      removeOnFail: { age: 3 * 24 * 3600, count: 50 },
    }
  );
  await queue.close();
  return { jobId: job.id };
}

module.exports = { start, triggerNow, QUEUE_NAME, JOB_DAILY, STATE_KEY };

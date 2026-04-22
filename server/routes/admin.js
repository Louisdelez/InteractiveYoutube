/**
 * Admin utility routes. Locked to loopback — a reverse proxy in front
 * must strip any X-Forwarded-For that claims to be 127.0.0.1, otherwise
 * a remote caller can impersonate localhost.
 */
const express = require('express');
const { Queue } = require('bullmq');
const { clearAllChatHistory } = require('../socket/chat');
const { getIO } = require('../socket');
const config = require('../config');
const log = require('../services/logger');
const { redis } = require('../services/redis');
const { createConnection } = require('../workers/bullmq-connection');
const { QUEUE_NAME, JOB_DAILY, STATE_KEY } = require('../workers/daily-maintenance');

const router = express.Router();

function loopbackOnly(req, res, next) {
  const ip = req.ip || req.connection?.remoteAddress || '';
  if (ip === '::1' || ip === '127.0.0.1' || ip === '::ffff:127.0.0.1') {
    return next();
  }
  res.status(403).json({ error: 'loopback only' });
}

/**
 * Force-resolve a stuck maintenance banner: wipes chat and emits
 * `maintenance:end`. Does NOT interact with the BullMQ worker —
 * useful when the banner got wedged by a client-side bug and not by
 * a real in-progress job.
 */
router.post('/maintenance-reset', loopbackOnly, async (req, res) => {
  try {
    const io = getIO();
    const cleared = await clearAllChatHistory(io);
    await redis.del(STATE_KEY);
    if (io) io.emit('maintenance:end');
    log.warn({ cleared }, 'admin: maintenance-reset forced');
    res.json({ ok: true, cleared, emitted: ['chat:cleared', 'maintenance:end'] });
  } catch (err) {
    log.error({ err: err.message }, 'admin: maintenance-reset failed');
    res.status(500).json({ error: err.message });
  }
});

/**
 * Enqueue a one-off maintenance run in BullMQ. The worker picks it up
 * immediately (delay:0). HTTP responds instantly with the jobId — the
 * actual maintenance runs asynchronously in the worker process.
 *
 * The web server is never restarted by this — the worker is the only
 * process that touches playlist state.
 */
router.post('/maintenance-trigger', loopbackOnly, async (req, res) => {
  try {
    const queue = new Queue(QUEUE_NAME, { connection: createConnection() });
    const job = await queue.add(
      JOB_DAILY,
      { manual: true, startedAt: new Date().toISOString() },
      {
        attempts: 1,
        removeOnComplete: { age: 3600, count: 10 },
        removeOnFail: { age: 3 * 24 * 3600, count: 50 },
      }
    );
    await queue.close();
    log.warn({ jobId: job.id }, 'admin: maintenance-trigger enqueued');
    res.json({
      ok: true,
      jobId: job.id,
      note: 'enqueued on BullMQ — worker will pick it up within seconds',
    });
  } catch (err) {
    log.error({ err: err.message }, 'admin: maintenance-trigger failed');
    res.status(500).json({ error: err.message });
  }
});

/**
 * Report the schedule state and most recent runs (read directly from
 * BullMQ in Redis — source of truth).
 */
router.get('/cron-status', loopbackOnly, async (req, res) => {
  let queue;
  try {
    queue = new Queue(QUEUE_NAME, { connection: createConnection() });
    const [schedulers, waiting, active, completed, failed] = await Promise.all([
      queue.getJobSchedulers(0, -1),
      queue.getWaitingCount(),
      queue.getActiveCount(),
      queue.getCompleted(0, 5),
      queue.getFailed(0, 5),
    ]);

    res.json({
      timezone: config.SERVER_TZ,
      serverTime: new Date().toISOString(),
      serverTimeLocal: new Date().toLocaleString('fr-FR', { timeZone: config.SERVER_TZ }),
      schedulers: schedulers.map((s) => ({
        key: s.key,
        pattern: s.pattern,
        tz: s.tz,
        next: s.next ? new Date(s.next).toISOString() : null,
      })),
      counts: { waiting, active },
      recentCompleted: completed.map((j) => ({
        id: j.id,
        name: j.name,
        finishedAt: j.finishedOn ? new Date(j.finishedOn).toISOString() : null,
        returnValue: j.returnvalue,
      })),
      recentFailed: failed.map((j) => ({
        id: j.id,
        name: j.name,
        failedAt: j.finishedOn ? new Date(j.finishedOn).toISOString() : null,
        reason: j.failedReason,
      })),
    });
  } catch (err) {
    log.error({ err: err.message }, 'admin: cron-status failed');
    res.status(500).json({ error: err.message });
  } finally {
    if (queue) await queue.close().catch(() => {});
  }
});

module.exports = router;

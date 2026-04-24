const express = require('express');
const status = require('../services/status');
const log = require('../services/logger');
const { t } = require('../i18n/fr');

const router = express.Router();

// Cache the global /status (live probe) for 5 s so a burst of page
// loads doesn't hammer YouTube / postgres / loki in lockstep.
let cache = { ts: 0, value: null };
const CACHE_MS = parseInt(process.env.STATUS_CACHE_MS) || 5000;
const HISTORY_DEFAULT_DAYS = parseInt(process.env.STATUS_HISTORY_DEFAULT_DAYS) || 90;

router.get('/', async (req, res) => {
  const now = Date.now();
  if (cache.value && now - cache.ts < CACHE_MS) {
    return res.json(cache.value);
  }
  try {
    const snapshot = await status.checkAll();
    cache = { ts: now, value: snapshot };
    res.json(snapshot);
  } catch (err) {
    log.error({ err: err.message }, 'status: live probe failed');
    res.status(500).json({ error: t('status.error.probe_failed') });
  }
});

router.get('/components', (req, res) => {
  res.json(status.COMPONENTS);
});

router.get('/history', async (req, res) => {
  const rawDays = parseInt(req.query.days, 10);
  const days = Number.isFinite(rawDays) && rawDays > 0 && rawDays <= 365 ? rawDays : HISTORY_DEFAULT_DAYS;
  try {
    const h = await status.historyByDay(days);
    res.json({ days, history: h });
  } catch (err) {
    log.error({ err: err.message }, 'status: history failed');
    res.status(500).json({ error: t('status.error.history_failed') });
  }
});

router.get('/incidents', async (req, res) => {
  try {
    const incidents = await status.listIncidents({ limit: 20 });
    res.json({ incidents });
  } catch (err) {
    log.error({ err: err.message }, 'status: incidents failed');
    res.status(500).json({ error: t('status.error.incidents_failed') });
  }
});

module.exports = router;

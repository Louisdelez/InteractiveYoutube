const express = require('express');
const { getUserSettings, setUserSettings } = require('../db');
const { requireAuth } = require('../middleware/auth');
const log = require('../services/logger');

const router = express.Router();

/**
 * Persistent user preferences (memory cache size, favourites). Anonymous
 * clients keep their settings locally; logged-in clients sync them
 * here so the same prefs follow them across machines.
 */
router.get('/settings', requireAuth, async (req, res) => {
  try {
    const settings = await getUserSettings(req.user.id);
    res.json({ settings });
  } catch (err) {
    log.error({ err: err.message, user: req.user.id }, 'get settings failed');
    res.status(500).json({ error: 'Erreur serveur.' });
  }
});

router.put('/settings', requireAuth, async (req, res) => {
  const settings = req.body && req.body.settings;
  if (!settings || typeof settings !== 'object') {
    return res.status(400).json({ error: 'Settings invalides.' });
  }
  // Light validation so we don't store nonsense.
  const safe = {};
  if (typeof settings.memory_capacity === 'number') {
    safe.memory_capacity = Math.max(0, Math.min(5, Math.floor(settings.memory_capacity)));
  }
  if (Array.isArray(settings.favorites)) {
    safe.favorites = settings.favorites.filter((s) => typeof s === 'string').slice(0, 50);
  }
  try {
    await setUserSettings(req.user.id, safe);
    res.json({ ok: true, settings: safe });
  } catch (err) {
    log.error({ err: err.message, user: req.user.id }, 'put settings failed');
    res.status(500).json({ error: 'Erreur serveur.' });
  }
});

module.exports = router;

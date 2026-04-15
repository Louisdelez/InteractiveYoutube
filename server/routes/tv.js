const express = require('express');
const config = require('../config');
const { getTvState } = require('../services/tv');

const router = express.Router();

router.get('/state', (req, res) => {
  const channelId = req.query.channel || config.CHANNELS[0].id;
  const state = getTvState(channelId);
  if (!state) {
    return res.status(503).json({ error: 'Playlist not ready' });
  }
  res.json(state);
});

router.get('/channels', (req, res) => {
  res.json(
    config.CHANNELS.map((c) => ({
      id: c.id,
      name: c.name,
      handle: c.handle || '',
      avatar: c.avatar || '',
    }))
  );
});

// Where the web fallback tells non-embed viewers to go.
router.get('/desktop-download', (req, res) => {
  res.json({ url: config.DESKTOP_DOWNLOAD_URL });
});

module.exports = router;

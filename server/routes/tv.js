const express = require('express');
const config = require('../config');
const { getTvState } = require('../services/tv');
const { getPlaylist } = require('../services/playlist');

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

// Raw playlist layout for a channel — lets the client build a TV-guide
// style schedule by playing the `(now - tvStartedAt) mod totalDuration`
// cycle forward from any wallclock timestamp. Returns only the fields
// needed for scheduling (trimmed from ~30 KB to ~5 KB per channel).
router.get('/playlist', (req, res) => {
  const channelId = req.query.channel || config.CHANNELS[0].id;
  const playlist = getPlaylist(channelId);
  if (!playlist) return res.status(503).json({ error: 'Playlist not ready' });
  res.json({
    channelId,
    tvStartedAt: playlist.tvStartedAt,
    totalDuration: playlist.totalDuration,
    videos: playlist.videos.map((v) => ({
      videoId: v.videoId,
      title: v.title,
      duration: v.duration,
    })),
  });
});

module.exports = router;

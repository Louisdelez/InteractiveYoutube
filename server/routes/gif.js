const express = require('express');
const config = require('../config');
const { t } = require('../i18n/fr');

const router = express.Router();

const TENOR_BASE = process.env.TENOR_API_BASE || 'https://g.tenor.com/v2';
const MEDIA_FILTER = process.env.TENOR_MEDIA_FILTER || 'gif,tinygif,nanogif';
const LIMIT = parseInt(process.env.TENOR_RESULT_LIMIT) || 30;

function mapResults(results) {
  return (results || []).map((r) => {
    const tiny = r.media_formats?.tinygif || r.media_formats?.nanogif || r.media_formats?.gif;
    const full = r.media_formats?.gif || tiny;
    return {
      id: r.id,
      title: r.title || '',
      gif_url: full?.url || '',
      preview_url: tiny?.url || '',
      width: tiny?.dims?.[0] || 100,
      height: tiny?.dims?.[1] || 100,
    };
  }).filter((g) => g.gif_url);
}

router.get('/trending', async (req, res) => {
  try {
    const url = `${TENOR_BASE}/featured?key=${config.TENOR_API_KEY}&limit=${LIMIT}&media_filter=${MEDIA_FILTER}&contentfilter=medium`;
    const resp = await fetch(url);
    if (!resp.ok) return res.status(502).json({ error: t('gif.error.tenor_http').replace('{status}', resp.status) });
    const data = await resp.json();
    res.json(mapResults(data.results));
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

router.get('/search', async (req, res) => {
  const q = req.query.q;
  if (!q || typeof q !== 'string') return res.json([]);
  try {
    const url = `${TENOR_BASE}/search?key=${config.TENOR_API_KEY}&q=${encodeURIComponent(q)}&limit=${LIMIT}&media_filter=${MEDIA_FILTER}&contentfilter=medium`;
    const resp = await fetch(url);
    if (!resp.ok) return res.status(502).json({ error: t('gif.error.tenor_http').replace('{status}', resp.status) });
    const data = await resp.json();
    res.json(mapResults(data.results));
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;

// Also export a sticker list router for the web client
const stickerRouter = express.Router();
const fs = require('fs');
const stickerDir = require('path').join(__dirname, '..', '..', 'desktop', 'assets', 'stickers');

stickerRouter.get('/list', (req, res) => {
  try {
    const files = fs.readdirSync(stickerDir)
      .filter((f) => f.endsWith('.png') || f.endsWith('.gif'))
      .sort();
    res.json(files);
  } catch {
    res.json([]);
  }
});

module.exports.stickerRouter = stickerRouter;

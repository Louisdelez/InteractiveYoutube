/**
 * Deezer Chart API — fetches the current top tracks.
 *
 * No auth required (Deezer's public-catalog endpoints are open).
 * Used by `HitsFeedChannel` to populate the "Hits du Moment" channel.
 * Country defaults to whatever Deezer infers from the server's IP ;
 * set `DEEZER_CHART_ID` to target a specific editorial chart
 * (0 = global, per Deezer convention).
 *
 * Rate limit: ~50 req / 5 s per IP. We hit this once per channel
 * refresh, so cost is negligible.
 *
 * Doc : https://developers.deezer.com/api/chart
 */
const log = require('./logger').child({ component: 'deezer' });

const CHART_BASE = process.env.DEEZER_API_BASE || 'https://api.deezer.com';
const CHART_TIMEOUT_MS = parseInt(process.env.DEEZER_TIMEOUT_MS) || 10_000;

/**
 * Fetch the top-tracks chart. Returns an array of simplified objects :
 *
 *   { title, artist, album, deezerId, cover }
 *
 * Skips entries missing title/artist. `limit` is a soft cap — Deezer
 * itself caps at 100 per request on `/chart/0/tracks`.
 */
async function fetchTopHits(limit = 50, chartId = 0) {
  const cap = Math.max(1, Math.min(100, limit));
  const url = `${CHART_BASE}/chart/${chartId}/tracks?limit=${cap}`;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), CHART_TIMEOUT_MS);
  let resp;
  try {
    resp = await fetch(url, { signal: controller.signal });
  } finally {
    clearTimeout(timeout);
  }
  if (!resp.ok) {
    throw new Error(`deezer chart HTTP ${resp.status}`);
  }
  const body = await resp.json();
  const rows = Array.isArray(body && body.data) ? body.data : [];
  const tracks = rows
    .map((t) => ({
      title: t.title_short || t.title || '',
      artist: (t.artist && t.artist.name) || '',
      album: (t.album && t.album.title) || '',
      deezerId: t.id,
      cover: (t.album && (t.album.cover_medium || t.album.cover)) || null,
    }))
    .filter((t) => t.title && t.artist);
  log.info({ chartId, count: tracks.length }, 'deezer: top hits fetched');
  return tracks;
}

module.exports = { fetchTopHits };

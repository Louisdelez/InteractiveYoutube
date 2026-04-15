/**
 * Scrape YouTube channel tabs to count videos, shorts, and lives
 * WITHOUT using the YouTube Data API (no quota consumed)
 *
 * Uses the channel page HTML which contains video counts in the tabs
 *
 * Usage: node scripts/scrape-audit.js
 */

const https = require('https');
const fs = require('fs');
const path = require('path');
const config = require('../config');

// Fetch a YouTube page and extract data from the initial HTML (no browser needed)
function fetchPage(url) {
  return new Promise((resolve, reject) => {
    const req = https.request(url, {
      headers: {
        'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
        'Accept-Language': 'fr-FR,fr;q=0.9,en;q=0.8',
      },
      timeout: 15000,
    }, (res) => {
      let data = '';
      res.on('data', chunk => data += chunk);
      res.on('end', () => resolve(data));
    });
    req.on('error', reject);
    req.on('timeout', () => { req.destroy(); reject(new Error('timeout')); });
    req.end();
  });
}

// Extract video count from a YouTube channel tab page
// YouTube embeds JSON data in the initial HTML with ytInitialData
function extractVideoCount(html) {
  // Look for the video count in tab content
  // YouTube puts total results in richGridRenderer or gridRenderer
  const match = html.match(/"totalResults"\s*:\s*(\d+)/);
  if (match) return parseInt(match[1]);

  // Alternative: count video entries in the JSON
  const videoMatches = html.match(/"videoId"\s*:\s*"[A-Za-z0-9_-]{11}"/g);
  return videoMatches ? videoMatches.length : 0;
}

// Get channel handle from channel ID by fetching the channel page
async function getChannelHandle(channelId) {
  try {
    const html = await fetchPage(`https://www.youtube.com/channel/${channelId}`);
    const match = html.match(/"canonicalBaseUrl"\s*:\s*"\/@([^"]+)"/);
    return match ? match[1] : null;
  } catch {
    return null;
  }
}

// Count videos on a specific tab (videos, shorts, streams)
async function countTab(handle, tab) {
  try {
    const url = `https://www.youtube.com/@${handle}/${tab}`;
    const html = await fetchPage(url);

    // Method 1: look for video count metadata
    // YouTube includes video count in the page data
    const countMatch = html.match(/"videoCountText"\s*:\s*\{[^}]*"text"\s*:\s*"([\d\s,.]+)/);
    if (countMatch) {
      return parseInt(countMatch[1].replace(/[\s,.]/g, ''));
    }

    // Method 2: count unique videoIds in the page (initial load = ~30 videos)
    const videoIds = new Set();
    const regex = /"videoId"\s*:\s*"([A-Za-z0-9_-]{11})"/g;
    let m;
    while ((m = regex.exec(html)) !== null) {
      videoIds.add(m[1]);
    }

    // Method 3: check for "no content" indicators
    if (html.includes('"messageText"') && videoIds.size < 3) {
      return 0;
    }

    // For the initial page load, YouTube shows ~30 videos
    // If we see 30, there are likely more (need scroll)
    // We'll mark it as "30+"
    return videoIds.size;
  } catch (err) {
    return -1; // error
  }
}

// More accurate: parse ytInitialData JSON
async function getChannelStats(handle) {
  try {
    const html = await fetchPage(`https://www.youtube.com/@${handle}`);

    // Extract subscriber count and total video count from about section
    const videoCountMatch = html.match(/"videoCountText"\s*:\s*\{"simpleText"\s*:\s*"([\d\s,.]+)\s/);
    const videoCount = videoCountMatch ? parseInt(videoCountMatch[1].replace(/[\s,.]/g, '')) : null;

    // Extract from channel header
    const videosMatch = html.match(/(\d[\d\s,.]*)\s*vid[ée]/i);
    const totalVideos = videosMatch ? parseInt(videosMatch[1].replace(/[\s,.]/g, '')) : null;

    return { totalVideos: videoCount || totalVideos };
  } catch {
    return { totalVideos: null };
  }
}

async function auditChannel(ch) {
  const handles = [];

  // Get handles for each YouTube channel ID
  for (const ytId of (ch.youtubeChannelIds || [])) {
    const handle = await getChannelHandle(ytId);
    if (handle) handles.push(handle);
    await sleep(500); // Rate limit
  }

  if (handles.length === 0) {
    return { name: ch.name, id: ch.id, error: 'No handles found' };
  }

  let totalVideos = 0, totalShorts = 0, totalStreams = 0;

  for (const handle of handles) {
    const videos = await countTab(handle, 'videos');
    await sleep(500);
    const shorts = await countTab(handle, 'shorts');
    await sleep(500);
    const streams = await countTab(handle, 'streams');
    await sleep(500);

    if (videos >= 0) totalVideos += videos;
    if (shorts >= 0) totalShorts += shorts;
    if (streams >= 0) totalStreams += streams;

    process.stderr.write(`  @${handle}: videos=${videos} shorts=${shorts} streams=${streams}\n`);
  }

  // Get cache count
  const file = path.join(__dirname, '..', 'data', `playlist-${ch.id}.json`);
  let cache = 0;
  try { cache = JSON.parse(fs.readFileSync(file, 'utf8')).videos.length; } catch(e) { cache = -1; }

  return {
    name: ch.name, id: ch.id, handles,
    videos: totalVideos, shorts: totalShorts, streams: totalStreams,
    total: totalVideos + totalShorts + totalStreams,
    cache,
  };
}

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

async function main() {
  const channels = config.CHANNELS.filter(c => !c.ordered);

  console.log('=== SCRAPE AUDIT (no API) ===\n');

  const results = [];
  for (const ch of channels) {
    process.stderr.write(`\nAuditing: ${ch.name}...\n`);
    const r = await auditChannel(ch);
    results.push(r);
  }

  console.log('| Chaîne | Vidéos (tab) | Shorts (tab) | Streams (tab) | Total YT | Cache | Diff |');
  console.log('|--------|-------------|-------------|--------------|----------|-------|------|');
  for (const r of results) {
    if (r.error) {
      console.log(`| ${r.name} | ERROR | - | - | - | - | ${r.error} |`);
    } else {
      const diff = r.cache >= 0 ? r.cache - r.videos : '?';
      console.log(`| ${r.name} | ${r.videos} | ${r.shorts} | ${r.streams} | ${r.total} | ${r.cache} | ${diff >= 0 ? '+' : ''}${diff} |`);
    }
  }
}

main().catch(console.error);

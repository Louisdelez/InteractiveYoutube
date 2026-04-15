/**
 * Complete channel audit script
 * Fetches ALL videos from ALL channels and categorizes them precisely:
 * - Normal videos (embeddable, not short, not live)
 * - Shorts (detected via HEAD request)
 * - Lives/Rediffs (has liveStreamingDetails)
 * - Non-embeddable
 * - Duration 0 (broken/processing)
 *
 * Usage: node scripts/audit-channels.js
 */

const https = require('https');
const config = require('../config');
const fs = require('fs');
const path = require('path');

const KEY = config.YOUTUBE_API_KEY;

function checkIsShort(id) {
  return new Promise((resolve) => {
    const req = https.request(`https://www.youtube.com/shorts/${id}`, { method: 'HEAD', timeout: 5000 }, (res) => {
      resolve(res.statusCode === 200);
    });
    req.on('error', () => resolve(false));
    req.on('timeout', () => { req.destroy(); resolve(false); });
    req.end();
  });
}

async function fetchAllVideoIds(ytChannelId) {
  const uploadsId = 'UU' + ytChannelId.substring(2);
  const ids = [];
  let pageToken = '';
  do {
    const url = `https://www.googleapis.com/youtube/v3/playlistItems?part=contentDetails&playlistId=${uploadsId}&maxResults=50&key=${KEY}${pageToken ? '&pageToken=' + pageToken : ''}`;
    const res = await fetch(url);
    const data = await res.json();
    if (!data.items || data.items.length === 0) break;
    ids.push(...data.items.map(i => i.contentDetails.videoId));
    pageToken = data.nextPageToken || '';
  } while (pageToken);
  return ids;
}

async function fetchVideoDetails(videoIds) {
  const results = [];
  for (let i = 0; i < videoIds.length; i += 50) {
    const batch = videoIds.slice(i, i + 50).join(',');
    const url = `https://www.googleapis.com/youtube/v3/videos?part=contentDetails,status,snippet,liveStreamingDetails&id=${batch}&key=${KEY}`;
    const res = await fetch(url);
    const data = await res.json();
    if (data.items) results.push(...data.items);
    // Rate limit: small delay between batches
    if (i + 50 < videoIds.length) await new Promise(r => setTimeout(r, 100));
  }
  return results;
}

async function filterShorts(videoIds, concurrency = 20) {
  const results = new Map();
  for (let i = 0; i < videoIds.length; i += concurrency) {
    const batch = videoIds.slice(i, i + concurrency);
    const checks = await Promise.all(batch.map(async (id) => {
      const isShort = await checkIsShort(id);
      return { id, isShort };
    }));
    for (const { id, isShort } of checks) {
      results.set(id, isShort);
    }
    if ((i + concurrency) % 200 === 0) {
      process.stderr.write(`  Shorts check: ${Math.min(i + concurrency, videoIds.length)}/${videoIds.length}\n`);
    }
  }
  return results;
}

async function auditChannel(ch) {
  process.stderr.write(`\nAuditing: ${ch.name} (${ch.id})...\n`);

  if (ch.ordered) {
    const file = path.join(__dirname, '..', 'data', `playlist-${ch.id}.json`);
    let cache = 0;
    try { cache = JSON.parse(fs.readFileSync(file, 'utf8')).videos.length; } catch(e) {}
    return { name: ch.name, id: ch.id, ordered: true, cache };
  }

  // 1. Fetch all video IDs from all source channels
  let allIds = [];
  for (const ytId of (ch.youtubeChannelIds || [])) {
    const ids = await fetchAllVideoIds(ytId);
    allIds = allIds.concat(ids);
    process.stderr.write(`  Fetched ${ids.length} IDs from ${ytId}\n`);
  }
  if (ch.extraPlaylists) {
    for (const plId of ch.extraPlaylists) {
      let pageToken = '';
      do {
        const url = `https://www.googleapis.com/youtube/v3/playlistItems?part=contentDetails&playlistId=${plId}&maxResults=50&key=${KEY}${pageToken ? '&pageToken=' + pageToken : ''}`;
        const res = await fetch(url);
        const data = await res.json();
        if (!data.items) break;
        allIds.push(...data.items.map(i => i.contentDetails.videoId));
        pageToken = data.nextPageToken || '';
      } while (pageToken);
    }
  }
  allIds = [...new Set(allIds)];
  process.stderr.write(`  Total unique IDs: ${allIds.length}\n`);

  // 2. Fetch details for all videos
  const details = await fetchVideoDetails(allIds);
  process.stderr.write(`  Got details for ${details.length} videos\n`);

  // 3. Categorize
  let notEmbeddable = 0;
  let lives = 0;
  let dur0 = 0;
  const embeddableNonLiveIds = [];

  for (const item of details) {
    const dur = item.contentDetails.duration;
    const emb = item.status.embeddable;
    const isLive = !!item.liveStreamingDetails;
    const isLiveNow = item.snippet.liveBroadcastContent === 'live' || item.snippet.liveBroadcastContent === 'upcoming';

    if (!emb) {
      notEmbeddable++;
    } else if (isLive || isLiveNow) {
      lives++;
    } else if (dur === 'P0D' || dur === 'PT0S') {
      dur0++;
    } else {
      embeddableNonLiveIds.push(item.id);
    }
  }

  // 4. Check Shorts via HEAD requests (only for embeddable non-live videos)
  process.stderr.write(`  Checking ${embeddableNonLiveIds.length} videos for Shorts...\n`);
  const shortsMap = await filterShorts(embeddableNonLiveIds);
  let shorts = 0;
  let normalVideos = 0;
  for (const [id, isShort] of shortsMap) {
    if (isShort) shorts++;
    else normalVideos++;
  }

  // 5. Get cache count
  const file = path.join(__dirname, '..', 'data', `playlist-${ch.id}.json`);
  let cache = 0;
  try { cache = JSON.parse(fs.readFileSync(file, 'utf8')).videos.length; } catch(e) { cache = -1; }

  const diff = cache >= 0 ? cache - normalVideos : '?';
  const status = cache < 0 ? 'NO CACHE' :
    (Math.abs(cache - normalVideos) <= Math.max(5, normalVideos * 0.05)) ? 'OK' : 'MISMATCH';

  return {
    name: ch.name, id: ch.id,
    ytTotal: allIds.length,
    notEmbeddable, lives, dur0, shorts, normalVideos,
    cache, diff, status,
    includeLives: !!ch.includeLives
  };
}

async function main() {
  console.log('=== COMPLETE CHANNEL AUDIT ===\n');

  const results = [];
  for (const ch of config.CHANNELS) {
    try {
      results.push(await auditChannel(ch));
    } catch (err) {
      process.stderr.write(`  ERROR: ${err.message}\n`);
      results.push({ name: ch.name, id: ch.id, status: 'ERROR: ' + err.message });
    }
  }

  // Print table
  console.log('| Chaîne | YT Total | Non-Emb | Lives | Shorts | Vidéos normales | Cache | Diff | Status |');
  console.log('|--------|----------|---------|-------|--------|----------------|-------|------|--------|');
  for (const r of results) {
    if (r.ordered) {
      console.log(`| ${r.name} | ORDERED | - | - | - | - | ${r.cache} | - | OK |`);
    } else if (r.status?.startsWith('ERROR')) {
      console.log(`| ${r.name} | ? | ? | ? | ? | ? | ? | ? | ${r.status} |`);
    } else {
      const livesNote = r.includeLives ? `${r.lives} (inclus)` : `${r.lives}`;
      console.log(`| ${r.name} | ${r.ytTotal} | ${r.notEmbeddable} | ${livesNote} | ${r.shorts} | ${r.normalVideos} | ${r.cache} | ${r.diff > 0 ? '+' : ''}${r.diff} | ${r.status} |`);
    }
  }

  // Summary
  const problems = results.filter(r => r.status === 'MISMATCH');
  console.log(`\nTotal channels: ${results.length}`);
  console.log(`OK: ${results.filter(r => r.status === 'OK').length}`);
  console.log(`MISMATCH: ${problems.length}`);
  if (problems.length > 0) {
    console.log('\nProblèmes à corriger:');
    for (const p of problems) {
      console.log(`  - ${p.name}: cache=${p.cache}, attendu=${p.normalVideos}, diff=${p.diff}`);
    }
  }
}

main().catch(console.error);

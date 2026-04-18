/**
 * ═══════════════════════════════════════════════════════════════
 *  MAINTENANCE AUTOMATIQUE — tous les jours à 3h du matin
 * ═══════════════════════════════════════════════════════════════
 *
 *  Ordre d'exécution (strictement séquentiel, chaque étape
 *  AWAIT la précédente — aucun process.exit ne peut interrompre
 *  une opération en cours) :
 *
 *  1. Mise à jour yt-dlp (le binaire serveur)
 *  2. Refresh playlists (1/7 des chaînes par jour, rotation)
 *  3. Nettoyage Redis (viewers stales, rate-limit keys expirés)
 *  4. Clear chat history (toutes les chaînes)
 *  5. Broadcast chat:cleared aux clients connectés
 *  6. Flush logs + attente 5s (Socket.IO flush)
 *  7. Restart process (PM2/nodemon respawn)
 *
 *  Le RSS poll (nouvelles vidéos) tourne séparément toutes les
 *  30 minutes, indépendamment de la maintenance.
 * ═══════════════════════════════════════════════════════════════
 */

const fs = require('fs');
const cron = require('node-cron');
const config = require('../config');
const { refreshPlaylist, addNewVideos } = require('../services/playlist');
const { fetchVideoDetails } = require('../services/youtube');
const { checkForNewUploads } = require('../services/rss');
const { queuePriorityVideo, getTvState } = require('../services/tv');
const { getIO } = require('../socket');
const { clearAllChatHistory } = require('../socket/chat');
const { redis } = require('../services/redis');
const ytdlpUpdater = require('../services/ytdlp-updater');
const log = require('../services/logger');

const LOG_FILE = '/tmp/koala-cron.log';
const POPCORN_MIN_DURATION = 5400; // 1h30

function cronLog(msg) {
  const line = `[${new Date().toISOString()}] ${msg}`;
  console.log(`[MAINTENANCE] ${msg}`);
  try { fs.appendFileSync(LOG_FILE, line + '\n'); } catch {}
}

// ─── Daily 3am maintenance ─────────────────────────────────────

async function dailyMaintenance() {
  const startTime = Date.now();
  cronLog('══════════════════════════════════════════');
  cronLog('MAINTENANCE START');
  cronLog('══════════════════════════════════════════');

  // Notify all clients that maintenance is starting
  try {
    const io = getIO();
    if (io) io.emit('maintenance:start');
  } catch {}

  // ── Step 1: Update yt-dlp ──────────────────────────────────
  cronLog('[1/7] Mise à jour yt-dlp...');
  try {
    await ytdlpUpdater.selfUpdate();
    cronLog('[1/7] yt-dlp OK');
  } catch (err) {
    cronLog(`[1/7] yt-dlp FAILED: ${err.message}`);
  }

  // ── Step 2: Refresh playlists (1/7 des chaînes) ────────────
  const day = new Date().getDay();
  const bucket = config.CHANNELS.filter((_, i) => i % 7 === day);
  cronLog(`[2/7] Refresh playlists: jour=${day}, ${bucket.length}/${config.CHANNELS.length} chaînes`);
  let refreshed = 0, refreshFailed = 0;
  for (const channel of bucket) {
    try {
      await refreshPlaylist(channel.id);
      const io = getIO();
      if (io) {
        io.to(`channel:${channel.id}`).emit('tv:refreshed');
        io.emit('tv:playlistUpdated', { channelId: channel.id });
      }
      refreshed++;
    } catch (err) {
      refreshFailed++;
      cronLog(`[2/7]   FAILED ${channel.id}: ${err.message}`);
    }
  }
  cronLog(`[2/7] Refresh done: ${refreshed} OK, ${refreshFailed} failed`);

  // ── Step 3: Nettoyage Redis ────────────────────────────────
  cronLog('[3/7] Nettoyage Redis...');
  try {
    // Clean stale viewer sets
    let cleanedViewers = 0;
    for (const ch of config.CHANNELS) {
      const key = `viewers:set:${ch.id}`;
      const count = await redis.scard(key);
      if (count > 0) {
        // At 3am, very few real viewers — clear stale entries
        const members = await redis.smembers(key);
        if (members.length > 0) {
          const io = getIO();
          if (io) {
            const liveSockets = await io.fetchSockets();
            const liveIds = new Set(liveSockets.map(s => s.id));
            const stale = members.filter(id => !liveIds.has(id));
            if (stale.length > 0) {
              await redis.srem(key, ...stale);
              cleanedViewers += stale.length;
            }
          }
        }
      }
    }
    // Clean expired rate-limit keys
    let cleanedRateKeys = 0;
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
    cronLog(`[3/7] Redis cleanup: ${cleanedViewers} stale viewers, ${cleanedRateKeys} expired rate keys`);
  } catch (err) {
    cronLog(`[3/7] Redis cleanup FAILED: ${err.message}`);
  }

  // ── Step 4: Clear ALL chat history ─────────────────────────
  cronLog('[4/7] Clear chat history...');
  try {
    const cleared = await clearAllChatHistory(getIO());
    cronLog(`[4/7] Chat cleared: ${cleared} channel histories wiped`);
  } catch (err) {
    cronLog(`[4/7] Chat clear FAILED: ${err.message}`);
  }

  // ── Step 5: Verify chat is actually empty ──────────────────
  cronLog('[5/7] Vérification...');
  try {
    let remaining = [];
    let cursor = '0';
    do {
      const [next, batch] = await redis.scan(cursor, 'MATCH', 'chat:history:*', 'COUNT', 500);
      cursor = next;
      remaining.push(...batch);
    } while (cursor !== '0');
    if (remaining.length > 0) {
      cronLog(`[5/7] WARNING: ${remaining.length} chat keys still present — force deleting`);
      await redis.del(...remaining);
      cronLog('[5/7] Force delete done');
    } else {
      cronLog('[5/7] Vérifié: chat history vide');
    }
  } catch (err) {
    cronLog(`[5/7] Vérification FAILED: ${err.message}`);
  }

  // ── Step 6: Broadcast to clients ───────────────────────────
  cronLog('[6/7] Broadcast chat:cleared...');
  try {
    const io = getIO();
    if (io) {
      io.emit('chat:cleared');
      cronLog('[6/7] Broadcast envoyé');
    } else {
      cronLog('[6/7] WARNING: io is null, no broadcast');
    }
  } catch (err) {
    cronLog(`[6/7] Broadcast FAILED: ${err.message}`);
  }

  // ── Step 7: Wait + restart ─────────────────────────────────
  const elapsed = Math.round((Date.now() - startTime) / 1000);
  cronLog(`[7/7] Maintenance terminée en ${elapsed}s — restart dans 5s`);
  cronLog('══════════════════════════════════════════');
  cronLog('MAINTENANCE COMPLETE');
  cronLog('══════════════════════════════════════════');

  // Notify clients maintenance is over (they'll reconnect after restart)
  try {
    const io = getIO();
    if (io) io.emit('maintenance:end');
  } catch {}

  // 5 secondes pour que Socket.IO flush tout aux clients
  await new Promise(resolve => setTimeout(resolve, 5000));

  // Exit 1 pour que PM2/nodemon respawn le process proprement
  process.exit(1);
}

// ─── RSS poll (toutes les 30 min, indépendant) ─────────────────

async function pollNormalChannel(channel) {
  let newIds = [];
  for (const ytId of channel.youtubeChannelIds) {
    const ids = await checkForNewUploads(channel.id, ytId);
    newIds = newIds.concat(ids);
  }
  newIds = [...new Set(newIds)];

  if (newIds.length > 0) {
    const newVideos = await fetchVideoDetails(newIds);
    if (newVideos.length > 0) {
      for (const video of newVideos) {
        queuePriorityVideo(channel.id, video);
      }
      addNewVideos(channel.id, newVideos);

      const io = getIO();
      if (io) {
        const freshState = getTvState(channel.id);
        if (freshState) {
          io.to(`channel:${channel.id}`).emit('tv:state', freshState);
        }
        io.emit('tv:playlistUpdated', { channelId: channel.id });
      }

      console.log(`[RSS:${channel.id}] ${newVideos.length} new video(s) added to playlist`);
    }
  }
}

async function pollOrderedChannel(channel) {
  const popcornYtChannelId = 'UCnyR4T5qpgOrWGcQU6Jinkw';
  const newIds = await checkForNewUploads(channel.id, popcornYtChannelId);
  if (newIds.length === 0) return;

  const newVideos = await fetchVideoDetails(newIds, { skipShortsFilter: true });
  const popcornEpisodes = newVideos.filter((v) => {
    return /popcorn/i.test(v.title) && v.duration >= POPCORN_MIN_DURATION;
  });

  if (popcornEpisodes.length === 0) return;

  addNewVideos(channel.id, popcornEpisodes);

  const io = getIO();
  if (io) {
    const freshState = getTvState(channel.id);
    if (freshState) {
      io.to(`channel:${channel.id}`).emit('tv:state', freshState);
    }
    io.emit('tv:playlistUpdated', { channelId: channel.id });
  }

  console.log(`[RSS:${channel.id}] ${popcornEpisodes.length} new Popcorn episode(s) added`);
}

// ─── Startup ───────────────────────────────────────────────────

function startCronJobs() {
  // 5 min before maintenance: warn all clients
  cron.schedule('55 2 * * *', () => {
    cronLog('Maintenance warning sent (5 min)');
    try {
      const io = getIO();
      if (io) io.emit('maintenance:warning', { minutes: 5 });
    } catch {}
  }, { timezone: config.SERVER_TZ });

  // Daily maintenance at 3am
  cron.schedule(config.DAILY_REFRESH_CRON, () => {
    dailyMaintenance().catch(err => {
      cronLog(`MAINTENANCE CRASHED: ${err.message}`);
      cronLog(err.stack || '');
      // Still restart even on crash — a fresh process is better
      // than a stuck one
      setTimeout(() => process.exit(1), 3000);
    });
  }, { timezone: config.SERVER_TZ });

  console.log(`[MAINTENANCE] Scheduled: ${config.DAILY_REFRESH_CRON} (${config.SERVER_TZ})`);

  // RSS poll every 30 minutes
  setInterval(async () => {
    for (const channel of config.CHANNELS) {
      try {
        if (channel.ordered) {
          await pollOrderedChannel(channel);
        } else {
          await pollNormalChannel(channel);
        }
      } catch (err) {
        console.error(`[RSS:${channel.id}] Poll error:`, err.message);
      }
    }
  }, config.RSS_POLL_INTERVAL_MS);

  console.log(`[MAINTENANCE] RSS poll: every ${config.RSS_POLL_INTERVAL_MS / 60000} min`);
}

module.exports = { startCronJobs };

/**
 * Worker entry point — runs separately from the web server.
 *
 * Responsibilities:
 *   1. Hydrate in-memory playlist state from disk (so refreshPlaylist
 *      has a baseline to diff against and doesn't re-bootstrap from
 *      the YouTube API)
 *   2. Keep yt-dlp binary fresh on the 6 h timer (same code path the
 *      web used to run — now it owns it exclusively)
 *   3. Arm the BullMQ daily-3 am scheduler + run the worker
 *   4. Run the 30-min RSS poll
 *
 * The web server no longer does any of this — it is pure HTTP/WS.
 * Worker crashes don't touch the web; web restarts don't affect the
 * cron schedule because BullMQ persists it in Redis.
 */
const log = require('./../services/logger');
const { initAllPlaylists } = require('../services/playlist');
const ytdlpUpdater = require('../services/ytdlp-updater');
const dailyMaintenance = require('./daily-maintenance');
const rssPoll = require('./rss-poll');

async function shutdown(signal) {
  log.info({ signal }, 'worker shutting down');
  rssPoll.stop();
  ytdlpUpdater.stop();
  // BullMQ workers/queues hold their own Redis sockets; letting the
  // process exit cleanly is enough — BullMQ's internal locks will
  // auto-release when the connection closes.
  process.exit(0);
}

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));
process.on('unhandledRejection', (reason) => {
  log.error({ reason: String(reason) }, 'worker: unhandled rejection');
});
process.on('uncaughtException', (err) => {
  log.fatal({ err: err.message, stack: err.stack }, 'worker: uncaught exception');
  // Unlike the web, we let the process die — PM2/systemd will respawn.
  process.exit(1);
});

async function start() {
  try {
    log.info('worker: initialising playlists');
    await initAllPlaylists();
    log.info('worker: playlists ready');

    ytdlpUpdater.start().catch((err) =>
      log.error({ err: err.message }, 'worker: yt-dlp updater failed to start')
    );

    await dailyMaintenance.start();
    rssPoll.start();

    log.info('worker: ready');
  } catch (err) {
    log.fatal({ err: err.message, stack: err.stack }, 'worker: failed to start');
    process.exit(1);
  }
}

start();

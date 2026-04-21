/**
 * Legacy shim — the real scheduler lives in server/workers/.
 *
 * Kept only as a light re-export so any older require() in the tree
 * keeps working. The web server does NOT register crons anymore.
 */
module.exports = {
  // No-op: left for backward compatibility with any stray caller.
  startCronJobs() {
    throw new Error(
      'startCronJobs() is deprecated — the scheduler now runs in server/workers/. ' +
      'Start the worker with: node server/workers (or via PM2 as app `koala-tv-maint`).'
    );
  },
};

/**
 * Prometheus metrics. Scrape via `GET /metrics`.
 *
 * Naming: `iy_<area>_<thing>_<unit>` (Prometheus convention).
 */
const client = require('prom-client');

client.collectDefaultMetrics({ prefix: 'iy_' });

const viewersGauge = new client.Gauge({
  name: 'iy_viewers_per_channel',
  help: 'Concurrent viewers connected to a channel',
  labelNames: ['channel'],
});

const connectionsCounter = new client.Counter({
  name: 'iy_socket_connections_total',
  help: 'Total Socket.IO connections accepted',
});

const chatMessagesCounter = new client.Counter({
  name: 'iy_chat_messages_total',
  help: 'Total chat messages broadcast',
  labelNames: ['channel'],
});

const syncBroadcastDuration = new client.Histogram({
  name: 'iy_tv_sync_broadcast_duration_ms',
  help: 'Time to fan out tv:sync to all channels (ms)',
  buckets: [1, 5, 10, 25, 50, 100, 250, 500],
});

const authAttemptsCounter = new client.Counter({
  name: 'iy_auth_attempts_total',
  help: 'Authentication attempts',
  labelNames: ['kind', 'status'],
});

// ─── Maintenance (BullMQ worker) ───────────────────────────────
// The worker runs in a separate process, so prom-client's in-process
// registry can't see its counters. Instead, the worker writes three
// Redis keys at the end of every run, and `refreshMaintenanceFromRedis`
// (called from the web's /metrics handler) syncs them into the
// web-side gauges right before the Prometheus serialization.
//
// Alert on `time() - iy_maintenance_last_success_ts > 93600` (26 h)
// to catch a silently-skipped nightly run.
const maintenanceDuration = new client.Gauge({
  name: 'iy_maintenance_duration_seconds',
  help: 'Duration of the most recent daily maintenance run, seconds',
});

const maintenanceLastSuccess = new client.Gauge({
  name: 'iy_maintenance_last_success_ts',
  help: 'UNIX timestamp of the last successful daily maintenance run',
});

const maintenanceLastFailure = new client.Gauge({
  name: 'iy_maintenance_last_failure_ts',
  help: 'UNIX timestamp of the last failed daily maintenance run',
});

const MAINT_LAST_SUCCESS_KEY = 'koala:maint:last_success_ts';
const MAINT_LAST_FAILURE_KEY = 'koala:maint:last_failure_ts';
const MAINT_LAST_DURATION_KEY = 'koala:maint:last_duration_sec';

async function refreshMaintenanceFromRedis(redis) {
  try {
    const [ok, ko, dur] = await redis.mget(
      MAINT_LAST_SUCCESS_KEY,
      MAINT_LAST_FAILURE_KEY,
      MAINT_LAST_DURATION_KEY
    );
    if (ok) maintenanceLastSuccess.set(Number(ok));
    if (ko) maintenanceLastFailure.set(Number(ko));
    if (dur) maintenanceDuration.set(Number(dur));
  } catch {
    // Redis hiccup — keep last known values, don't blow the scrape up
  }
}

module.exports = {
  registry: client.register,
  viewersGauge,
  connectionsCounter,
  chatMessagesCounter,
  syncBroadcastDuration,
  authAttemptsCounter,
  maintenanceDuration,
  maintenanceLastSuccess,
  maintenanceLastFailure,
  refreshMaintenanceFromRedis,
  MAINT_LAST_SUCCESS_KEY,
  MAINT_LAST_FAILURE_KEY,
  MAINT_LAST_DURATION_KEY,
};

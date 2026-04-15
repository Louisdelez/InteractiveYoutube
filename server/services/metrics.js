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

module.exports = {
  registry: client.register,
  viewersGauge,
  connectionsCounter,
  chatMessagesCounter,
  syncBroadcastDuration,
  authAttemptsCounter,
};

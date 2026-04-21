const config = require('./config');
const express = require('express');
const http = require('http');
const cookieParser = require('cookie-parser');
const cors = require('cors');
const path = require('path');
const pinoHttp = require('pino-http');

const log = require('./services/logger');
const metrics = require('./services/metrics');
const { redis } = require('./services/redis');

const { initDB, shutdown: shutdownDB } = require('./db');
const { initAllPlaylists } = require('./services/playlist');
const { setupSocketIO, getIO, shutdown: shutdownSocket } = require('./socket');
const workerBridge = require('./socket/worker-bridge');

const authRoutes = require('./routes/auth');
const tvRoutes = require('./routes/tv');
const userRoutes = require('./routes/user');
const gifRoutes = require('./routes/gif');
const { stickerRouter } = require('./routes/gif');
const logsRoutes = require('./routes/logs');
const statusRoutes = require('./routes/status');
const statusService = require('./services/status');
const adminRoutes = require('./routes/admin');

const app = express();
const server = http.createServer(app);

// Trust proxy headers (X-Forwarded-For, etc.) — assume one hop in
// front (nginx / Cloud Run / etc.). Required for express-rate-limit
// to key by real client IP.
app.set('trust proxy', 1);

// Structured request logger (JSON in prod, pretty in dev).
app.use(pinoHttp({ logger: log }));

// Middleware
app.use(cors({
  origin: [config.CLIENT_ORIGIN, 'tauri://localhost', 'https://tauri.localhost'],
  credentials: true,
}));
app.use(express.json({ limit: '10kb' }));
app.use(cookieParser());

// Health check (kept lightweight — used by load balancers, also by
// the desktop app's connectivity ping).
app.get('/health', (req, res) => {
  res.json({ status: 'ok', uptime: process.uptime() });
});

// Prometheus metrics scrape endpoint.
app.get('/metrics', async (req, res) => {
  try {
    // Worker-side counters live in a different process — hydrate them
    // from Redis right before serializing so Prometheus sees fresh
    // values (the worker writes to these keys at the end of every run).
    await metrics.refreshMaintenanceFromRedis(redis);
    res.set('Content-Type', metrics.registry.contentType);
    res.end(await metrics.registry.metrics());
  } catch (err) {
    res.status(500).end(String(err));
  }
});

// Serve sticker assets (used by web chat to render [sticker:name] messages)
app.use('/stickers', express.static(path.join(__dirname, '..', 'desktop', 'assets', 'stickers'), {
  maxAge: '7d',
  immutable: true,
}));

// Routes
app.use('/api/auth', authRoutes);
app.use('/api/tv', tvRoutes);
app.use('/api/user', userRoutes);
app.use('/api/gifs', gifRoutes);
app.use('/api/stickers', stickerRouter);
app.use('/api/logs', logsRoutes);
// Status endpoints are public and read-only — allow any origin so the
// standalone status page (hosted on its own subdomain) can poll them
// without a CORS preflight failure.
app.use('/api/status', cors({ origin: '*', credentials: false }), statusRoutes);
app.use('/api/admin', adminRoutes);

// Serve client in production
if (config.NODE_ENV === 'production') {
  const clientDist = path.join(__dirname, '..', 'client', 'dist');
  app.use(express.static(clientDist, { maxAge: '1y', immutable: true }));
  app.get('*', (req, res) => {
    res.sendFile(path.join(clientDist, 'index.html'));
  });
}

// Global error handler
app.use((err, req, res, next) => {
  log.error({ err: err.message, stack: err.stack }, 'unhandled error');
  res.status(500).json({ error: 'Internal server error' });
});

// Graceful shutdown
function gracefulShutdown(signal) {
  log.info({ signal }, 'shutting down');
  workerBridge.stop();
  shutdownSocket();
  server.close(async () => {
    log.info('http server closed');
    await shutdownDB();
    process.exit(0);
  });
  // Force kill after 10s
  setTimeout(() => {
    log.error('forced shutdown after timeout');
    process.exit(1);
  }, 10000);
}

process.on('SIGTERM', () => gracefulShutdown('SIGTERM'));
process.on('SIGINT', () => gracefulShutdown('SIGINT'));
process.on('unhandledRejection', (reason) => {
  log.error({ reason: String(reason) }, 'unhandled rejection');
});
process.on('uncaughtException', (err) => {
  log.fatal({ err: err.message, stack: err.stack }, 'uncaught exception');
  gracefulShutdown('uncaughtException');
});

// Boot
async function start() {
  try {
    await initDB();

    // yt-dlp updates + scheduled maintenance + RSS poll all live in
    // the worker process (server/workers/*). The web server is pure
    // HTTP/WS — no setInterval, no cron, no disk writes that could
    // trigger nodemon restarts mid-operation.

    log.info('initialising playlists');
    await initAllPlaylists();
    log.info('all playlists ready');

    setupSocketIO(server);
    workerBridge.start(getIO());

    await statusService.initStatusSchema();
    statusService.startCollector();

    server.listen(config.PORT, () => {
      log.info({ port: config.PORT }, 'server listening');
    });
  } catch (err) {
    log.fatal({ err: err.message, stack: err.stack }, 'failed to start');
    process.exit(1);
  }
}

start();

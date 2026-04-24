const Redis = require('ioredis');
const config = require('../config');
const log = require('./logger').child({ component: 'redis' });

const REDIS_URL = config.REDIS_URL || 'redis://localhost:6379';

// Main client for commands
const RETRY_STEP_MS = parseInt(process.env.REDIS_RETRY_STEP_MS) || 100;
const RETRY_MAX_MS = parseInt(process.env.REDIS_RETRY_MAX_MS) || 3000;
const MAX_RETRIES = parseInt(process.env.REDIS_MAX_RETRIES_PER_REQUEST) || 3;
const redis = new Redis(REDIS_URL, {
  maxRetriesPerRequest: MAX_RETRIES,
  retryStrategy(times) {
    return Math.min(times * RETRY_STEP_MS, RETRY_MAX_MS);
  },
  lazyConnect: false,
});

redis.on('connect', () => log.info('connected'));
redis.on('error', (err) => log.error({ err: err.message }, 'redis error'));

// Pub client for Socket.IO adapter
const redisPub = new Redis(REDIS_URL);
// Sub client for Socket.IO adapter
const redisSub = new Redis(REDIS_URL);

module.exports = { redis, redisPub, redisSub };

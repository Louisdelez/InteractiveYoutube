const Redis = require('ioredis');
const config = require('../config');
const log = require('./logger').child({ component: 'redis' });

const REDIS_URL = config.REDIS_URL || 'redis://localhost:6379';

// Main client for commands
const redis = new Redis(REDIS_URL, {
  maxRetriesPerRequest: 3,
  retryStrategy(times) {
    const delay = Math.min(times * 100, 3000);
    return delay;
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

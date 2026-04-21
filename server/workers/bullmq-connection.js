/**
 * BullMQ requires an ioredis client with maxRetriesPerRequest:null and
 * no ready-check — it long-polls the queue with BRPOPLPUSH and the
 * usual defaults cause spurious "Command timed out" errors.
 *
 * Each Queue/Worker gets its own connection (BullMQ recommends not
 * sharing one socket between a queue and a worker).
 */
const IORedis = require('ioredis');
const config = require('../config');

function createConnection() {
  return new IORedis(config.REDIS_URL, {
    maxRetriesPerRequest: null,
    enableReadyCheck: false,
  });
}

module.exports = { createConnection };

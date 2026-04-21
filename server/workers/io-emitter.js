/**
 * Socket.IO emitter — lets the worker process send events to all
 * clients connected to the web server, via the Redis adapter they
 * both share. No HTTP server is attached here.
 *
 * The web server MUST use @socket.io/redis-adapter on the same Redis
 * instance for this to reach real clients — which it already does in
 * server/socket/index.js.
 */
const { Emitter } = require('@socket.io/redis-emitter');
const { redisPub } = require('../services/redis');

let emitter = null;

function getEmitter() {
  if (!emitter) emitter = new Emitter(redisPub);
  return emitter;
}

module.exports = { getEmitter };

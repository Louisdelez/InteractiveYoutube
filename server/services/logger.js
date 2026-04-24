/**
 * Centralised pino logger.
 *
 * Transports (all driven via pino.transport worker):
 *  - stdout: pino-pretty in dev, raw JSON in prod
 *  - logs/server.log.YYYY-MM-DD (all levels, daily rotation, 14d keep)
 *  - logs/server-error.log.YYYY-MM-DD (level >= error, daily rotation, 30d keep)
 *
 * Rotation is handled by pino-roll. Files land in <repo>/logs (created
 * on boot). Rotation survives reboots because the transport reopens
 * the current date file when the server restarts.
 */
const path = require('path');
const fs = require('fs');
const pino = require('pino');

const isDev = process.env.NODE_ENV !== 'production';
const level = process.env.LOG_LEVEL || (isDev ? 'debug' : 'info');

const logsDir = path.resolve(__dirname, '..', '..', 'logs');
fs.mkdirSync(logsDir, { recursive: true });

const rollCommon = {
  frequency: 'daily',
  mkdir: true,
  dateFormat: 'yyyy-MM-dd',
  size: '50m',
};

const targets = [
  {
    target: 'pino-roll',
    level,
    options: {
      ...rollCommon,
      file: path.join(logsDir, 'server.log'),
      limit: { count: 14 },
    },
  },
  {
    target: 'pino-roll',
    level: 'error',
    options: {
      ...rollCommon,
      file: path.join(logsDir, 'server-error.log'),
      limit: { count: 30 },
    },
  },
  isDev
    ? {
        target: 'pino-pretty',
        level,
        options: { colorize: true, singleLine: false, translateTime: 'HH:MM:ss.l', destination: 1 },
      }
    : { target: 'pino/file', level, options: { destination: 1 } },
];

const logger = pino(
  {
    level,
    base: { svc: 'koala-tv-server' },
    redact: ['req.headers.cookie', 'req.headers.authorization', '*.password', '*.passwordHash'],
  },
  pino.transport({ targets })
);

module.exports = logger;

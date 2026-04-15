/**
 * Centralised pino JSON logger. All modules import from here so we
 * have one place to configure level / transport / redaction.
 */
const pino = require('pino');

const isDev = process.env.NODE_ENV !== 'production';

const logger = pino({
  level: process.env.LOG_LEVEL || (isDev ? 'debug' : 'info'),
  base: { svc: 'iy-server' },
  redact: ['req.headers.cookie', 'req.headers.authorization', '*.password', '*.passwordHash'],
  ...(isDev && {
    transport: {
      target: 'pino-pretty',
      options: { colorize: true, singleLine: false, translateTime: 'HH:MM:ss.l' },
    },
  }),
});

module.exports = logger;

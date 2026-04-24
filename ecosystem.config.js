/**
 * PM2 process map — two apps:
 *
 *   koala-tv-web     pure HTTP/WS, cluster mode for CPU cores.
 *                    Never touches the scheduler or playlist writes.
 *
 *   koala-tv-maint   single-instance BullMQ worker. Owns:
 *                      - daily 3 am maintenance (yt-dlp + refresh + chat clear)
 *                      - 30-min RSS poll
 *                      - 6 h yt-dlp updater
 *                    State changes are relayed to the web via Redis
 *                    pub/sub (koala:*) so the web reloads from disk
 *                    and pushes fresh tv:state to its clients.
 *
 * The web and the maint worker share the same Redis. The maint worker
 * MUST be a single instance (concurrency: 1 inside BullMQ too) or
 * overlapping runs will double-clear the chat.
 */
// All secrets / connection strings come from the process environment
// (or a .env file loaded by server/config.js). PM2 passes through the
// parent env by default, so setting them in your shell / systemd unit
// / deploy pipeline is enough. Empty strings committed here used to
// shadow the real value — removed.
const sharedEnv = {
  SERVER_TZ: process.env.SERVER_TZ || 'Europe/Paris',
};

module.exports = {
  apps: [
    {
      name: 'koala-tv-web',
      script: 'server/index.js',
      instances: 'max',              // 1 per CPU core
      exec_mode: 'cluster',
      max_memory_restart: '600M',
      node_args: '--max-old-space-size=512',
      env_production: {
        NODE_ENV: 'production',
        PORT: 4500,
        ROLE: 'web',
        CLIENT_ORIGIN: 'https://yourdomain.com',
        ...sharedEnv,
      },
      kill_timeout: 10000,
      listen_timeout: 10000,
      shutdown_with_message: true,
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z',
      error_file: './logs/web-error.log',
      out_file: './logs/web-out.log',
      merge_logs: true,
      autorestart: true,
      watch: false,
      max_restarts: 10,
      restart_delay: 1000,
    },
    {
      name: 'koala-tv-maint',
      script: 'server/workers/index.js',
      instances: 1,                  // scheduler/worker must be a singleton
      exec_mode: 'fork',
      max_memory_restart: '500M',
      env_production: {
        NODE_ENV: 'production',
        ROLE: 'maint',
        // Optional: Healthchecks.io dead-man switch. Unset to disable.
        HEALTHCHECKS_URL: '',
        ...sharedEnv,
      },
      kill_timeout: 15000,           // BullMQ jobs may be mid-flight
      shutdown_with_message: true,
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z',
      error_file: './logs/maint-error.log',
      out_file: './logs/maint-out.log',
      merge_logs: true,
      autorestart: true,
      watch: false,
      max_restarts: 10,
      restart_delay: 2000,
    },
  ],
};

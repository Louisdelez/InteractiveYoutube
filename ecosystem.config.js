module.exports = {
  apps: [
    {
      name: 'koala-tv',
      script: 'server/index.js',
      instances: 'max', // 1 worker per CPU core
      exec_mode: 'cluster',
      max_memory_restart: '300M',
      node_args: '--max-old-space-size=512',
      env_production: {
        NODE_ENV: 'production',
        PORT: 4500,
        YOUTUBE_API_KEY: '',      // Set in .env or CI/CD
        YOUTUBE_CHANNEL_ID: '',
        JWT_SECRET: '',
        REDIS_URL: 'redis://localhost:6379',
        DATABASE_URL: 'postgresql://interactiveyoutube:interactiveyoutube@localhost:5432/interactiveyoutube',
        CLIENT_ORIGIN: 'https://yourdomain.com',
      },
      // Graceful shutdown
      kill_timeout: 10000,
      listen_timeout: 10000,
      shutdown_with_message: true,
      // Logs
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z',
      error_file: './logs/error.log',
      out_file: './logs/out.log',
      merge_logs: true,
      // Auto-restart
      autorestart: true,
      watch: false,
      max_restarts: 10,
      restart_delay: 1000,
    },
  ],
};

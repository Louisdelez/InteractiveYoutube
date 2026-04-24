require('dotenv').config({ path: require('path').join(__dirname, '..', '.env') });

const config = {
  YOUTUBE_API_KEY: process.env.YOUTUBE_API_KEY,
  JWT_SECRET: process.env.JWT_SECRET,
  PORT: parseInt(process.env.PORT) || 4500,
  CLIENT_ORIGIN: process.env.CLIENT_ORIGIN || 'http://localhost:4501',
  NODE_ENV: process.env.NODE_ENV || 'development',
  RSS_POLL_INTERVAL_MS: 30 * 60 * 1000,
  // Every day at 3am: refresh 1/7th of the channels + restart process.
  // Spreads API quota over the week and keeps the server fresh.
  DAILY_REFRESH_CRON: '0 3 * * *',
  // Timezone for the daily cron + chat message timestamps. Defaults
  // to the process TZ (`process.env.TZ` / host). Set explicitly in prod
  // (e.g. "Europe/Paris") so Docker containers don't drift to UTC.
  SERVER_TZ: process.env.SERVER_TZ || process.env.TZ || 'Europe/Paris',
  DRIFT_CORRECTION_INTERVAL_MS: 15000,
  CHAT_RATE_LIMIT_MS: 1000,
  CHAT_BUFFER_SIZE: 200,
  CHAT_BATCH_INTERVAL_MS: 150,
  CHAT_RATE_WINDOW_MS: 5000,
  CHAT_RATE_MAX_MESSAGES: 5,
  // Tenor v2 GIF API — uses a Google Cloud API key (same type as
  // YouTube). No fallback: the key is mandatory if the /api/gifs
  // routes are hit. Missing = boot refuses. Obtain a key from
  // https://console.cloud.google.com/ and `export TENOR_API_KEY=...`.
  TENOR_API_KEY: process.env.TENOR_API_KEY,
  REDIS_URL: process.env.REDIS_URL || 'redis://localhost:6379',
  // Postgres connection string — mandatory, no default credentials.
  // See `.env.example` and `docs/OPERATIONS.md` for the bootstrap.
  DATABASE_URL: process.env.DATABASE_URL,
  // Where the web fallback points users who hit a non-embeddable video.
  // Override via env when a release is published.
  DESKTOP_DOWNLOAD_URL:
    process.env.DESKTOP_DOWNLOAD_URL ||
    'https://github.com/Louisdelez/InteractiveYoutube/releases/latest',

  // Multi-channel configuration. Loaded from `server/config/channels.json`
  // — schema, per channel:
  //
  //   id                 : string, URL-safe slug (used as the Socket.IO room
  //                        name and the server-side state key)
  //   name               : display name
  //   handle             : YouTube @handle, used to deep-link to the creator
  //   youtubeChannelIds  : array of `UC…` IDs — videos from all merge into
  //                        one playlist. Mutually exclusive with `ordered`.
  //   avatar             : channel thumbnail (s160 googleusercontent URL),
  //                        or a local `/avatars/<slug>.jpg` path for hosts
  //                        that don't serve a public avatar.
  //   includeLives       : boolean — include ongoing live broadcasts in the
  //                        refresh (default false; stream-heavy channels).
  //   extraPlaylists     : array of playlist IDs to merge on top of the
  //                        channel uploads (for chronologically-curated mixes).
  //   ordered            : boolean — fixed play order, no shuffle, loop at
  //                        the end. Required companion: `fixedVideoIds` OR
  //                        `youtubePlaylists`.
  //   fixedVideoIds      : array of video IDs in play order.
  //   youtubePlaylists   : array of playlist IDs in play order; each playlist's
  //                        videos are concatenated in listing order.
  //
  // Edit the JSON file to add / remove / reorder channels. No code change
  // needed in this file to list a new chaîne. `loadAll` wraps each raw
  // entry in the appropriate `Channel` subclass so the rest of the
  // codebase iterates polymorphically (no `if (channel.ordered && …)`
  // dispatch — channel.fetchVideoIds() / channel.pollRss() instead).
  CHANNELS: require('./models/channel').loadAll(require('./config/channels.json')),
};

const required = [
  'YOUTUBE_API_KEY',
  'JWT_SECRET',
  'DATABASE_URL',
  'TENOR_API_KEY',
];
const missing = required.filter((k) => !config[k]);
if (missing.length) {
  console.error(
    `[config] missing required env vars: ${missing.join(', ')}\n` +
      `See .env.example and docs/OPERATIONS.md for setup.`
  );
  process.exit(1);
}

module.exports = config;

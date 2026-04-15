const { Pool } = require('pg');
const config = require('./config');

const pool = new Pool({
  connectionString: config.DATABASE_URL,
  max: 20, // Max connections in pool
  idleTimeoutMillis: 30000,
  connectionTimeoutMillis: 5000,
});

pool.on('error', (err) => {
  console.error('[DB] Unexpected pool error:', err.message);
});

// Initialize schema
async function initDB() {
  await pool.query(`
    CREATE TABLE IF NOT EXISTS users (
      id SERIAL PRIMARY KEY,
      username TEXT UNIQUE NOT NULL,
      email TEXT UNIQUE NOT NULL,
      password_hash TEXT NOT NULL,
      color TEXT NOT NULL DEFAULT '#1E90FF',
      settings JSONB NOT NULL DEFAULT '{}'::jsonb,
      created_at TIMESTAMPTZ DEFAULT NOW()
    )
  `);
  // Backfill the settings column on existing schemas (best-effort).
  await pool.query(
    `ALTER TABLE users ADD COLUMN IF NOT EXISTS settings JSONB NOT NULL DEFAULT '{}'::jsonb`
  );
  await pool.query(`CREATE INDEX IF NOT EXISTS idx_users_email ON users(email)`);
  await pool.query(`CREATE INDEX IF NOT EXISTS idx_users_username ON users(username)`);
  console.log('[DB] PostgreSQL initialized');
}

async function getUserSettings(userId) {
  const result = await pool.query('SELECT settings FROM users WHERE id = $1', [userId]);
  return result.rows[0]?.settings || {};
}

async function setUserSettings(userId, settings) {
  await pool.query('UPDATE users SET settings = $1 WHERE id = $2', [
    JSON.stringify(settings),
    userId,
  ]);
}

// Twitch-style vibrant chat colour assigned at registration. Random
// hue, saturation 60-90 %, lightness 55-75 %, returned as #RRGGBB.
function generateTwitchColor() {
  const h = Math.random() * 360;
  const s = (60 + Math.random() * 30) / 100;
  const l = (55 + Math.random() * 20) / 100;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const hp = h / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  let r, g, b;
  if (hp < 1) [r, g, b] = [c, x, 0];
  else if (hp < 2) [r, g, b] = [x, c, 0];
  else if (hp < 3) [r, g, b] = [0, c, x];
  else if (hp < 4) [r, g, b] = [0, x, c];
  else if (hp < 5) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  const m = l - c / 2;
  const toHex = (v) => Math.round((v + m) * 255).toString(16).padStart(2, '0').toUpperCase();
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

async function createUser(username, email, passwordHash) {
  const color = generateTwitchColor();
  const result = await pool.query(
    'INSERT INTO users (username, email, password_hash, color) VALUES ($1, $2, $3, $4) RETURNING id, username, email, color, created_at',
    [username, email, passwordHash, color]
  );
  return result.rows[0];
}

async function findUserByEmail(email) {
  const result = await pool.query('SELECT * FROM users WHERE email = $1', [email]);
  return result.rows[0] || null;
}

async function findUserById(id) {
  const result = await pool.query(
    'SELECT id, username, email, color, created_at FROM users WHERE id = $1',
    [id]
  );
  return result.rows[0] || null;
}

async function findUserByUsername(username) {
  const result = await pool.query('SELECT * FROM users WHERE username = $1', [username]);
  return result.rows[0] || null;
}

async function shutdown() {
  await pool.end();
}

module.exports = {
  pool,
  initDB,
  createUser,
  findUserByEmail,
  findUserById,
  findUserByUsername,
  getUserSettings,
  setUserSettings,
  shutdown,
};

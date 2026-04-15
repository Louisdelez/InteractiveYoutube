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

// Twitch-style chat colour assigned at registration — picked from the
// canonical 15-colour Twitch palette so the chat looks consistent with
// what users already know from Twitch.
const { generateColor: generateTwitchColor } = require('./services/pseudo');

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

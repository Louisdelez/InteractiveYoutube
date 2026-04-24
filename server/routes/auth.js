const express = require('express');
const bcrypt = require('bcrypt');
const jwt = require('jsonwebtoken');
const rateLimit = require('express-rate-limit');
const config = require('../config');
const { createUser, findUserByEmail, findUserByUsername } = require('../db');
const { optionalAuth } = require('../middleware/auth');
const log = require('../services/logger');
const metrics = require('../services/metrics');
const { t } = require('../i18n/fr');

const router = express.Router();

// Brute-force / credential-stuffing defence. bcrypt is CPU-bound (10
// rounds ≈ 100 ms), so unrate-limited login is also a DoS vector.
const loginLimiter = rateLimit({
  windowMs: 15 * 60 * 1000,
  max: 5,
  standardHeaders: 'draft-7',
  legacyHeaders: false,
  message: { error: t('auth.error.rate_limit_login') },
});

const registerLimiter = rateLimit({
  windowMs: 60 * 60 * 1000,
  max: 10,
  standardHeaders: 'draft-7',
  legacyHeaders: false,
  message: { error: t('auth.error.rate_limit_register') },
});

const COOKIE_OPTIONS = {
  httpOnly: true,
  secure: config.NODE_ENV === 'production',
  // `strict` blocks the cookie from being sent with cross-site
  // navigations entirely (CSRF defence — `lax` allows top-level
  // GET cross-site, which we don't need). Tauri/desktop traffic
  // sets the cookie via the same origin handshake, unaffected.
  sameSite: 'strict',
  maxAge: 7 * 24 * 60 * 60 * 1000, // 7 days
};

router.post('/register', registerLimiter, async (req, res) => {
  try {
    const { username, email, password } = req.body;

    if (!username || !email || !password) {
      return res.status(400).json({ error: t('auth.error.all_fields_required') });
    }

    if (username.length < 3 || username.length > 20) {
      return res.status(400).json({ error: t('auth.error.username_length') });
    }

    if (!/^[a-zA-Z0-9_-]+$/.test(username)) {
      return res.status(400).json({ error: t('auth.error.username_chars') });
    }

    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      return res.status(400).json({ error: t('auth.error.email_invalid') });
    }

    if (password.length < 6) {
      return res.status(400).json({ error: t('auth.error.password_length') });
    }

    if (await findUserByEmail(email)) {
      return res.status(400).json({ error: t('auth.error.email_taken') });
    }

    if (await findUserByUsername(username)) {
      return res.status(400).json({ error: t('auth.error.username_taken') });
    }

    const passwordHash = await bcrypt.hash(password, 10);
    const user = await createUser(username, email, passwordHash);
    const token = jwt.sign({ userId: user.id, username: user.username }, config.JWT_SECRET, { expiresIn: '7d' });

    res.cookie('token', token, COOKIE_OPTIONS);
    metrics.authAttemptsCounter.inc({ kind: 'register', status: 'ok' });
    res.json({ user: { id: user.id, username: user.username, color: user.color } });
  } catch (err) {
    log.error({ err: err.message }, 'register error');
    metrics.authAttemptsCounter.inc({ kind: 'register', status: 'error' });
    res.status(500).json({ error: t('auth.error.server') });
  }
});

router.post('/login', loginLimiter, async (req, res) => {
  try {
    const { email, password } = req.body;

    if (!email || !password) {
      return res.status(400).json({ error: t('auth.error.email_password_required') });
    }

    const user = await findUserByEmail(email);
    if (!user) {
      return res.status(401).json({ error: t('auth.error.bad_credentials') });
    }

    const valid = await bcrypt.compare(password, user.password_hash);
    if (!valid) {
      return res.status(401).json({ error: t('auth.error.bad_credentials') });
    }

    const token = jwt.sign({ userId: user.id, username: user.username }, config.JWT_SECRET, { expiresIn: '7d' });

    res.cookie('token', token, COOKIE_OPTIONS);
    metrics.authAttemptsCounter.inc({ kind: 'login', status: 'ok' });
    res.json({ user: { id: user.id, username: user.username, color: user.color } });
  } catch (err) {
    log.error({ err: err.message }, 'login error');
    metrics.authAttemptsCounter.inc({ kind: 'login', status: 'error' });
    res.status(500).json({ error: t('auth.error.server') });
  }
});

router.post('/logout', (req, res) => {
  res.clearCookie('token', COOKIE_OPTIONS);
  res.json({ ok: true });
});

router.get('/me', optionalAuth, (req, res) => {
  if (!req.user) {
    return res.status(401).json({ error: t('auth.error.not_authenticated') });
  }
  res.json({ user: { id: req.user.id, username: req.user.username, color: req.user.color } });
});

module.exports = router;

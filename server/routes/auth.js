const express = require('express');
const bcrypt = require('bcrypt');
const jwt = require('jsonwebtoken');
const rateLimit = require('express-rate-limit');
const config = require('../config');
const { createUser, findUserByEmail, findUserByUsername } = require('../db');
const { optionalAuth } = require('../middleware/auth');
const log = require('../services/logger');
const metrics = require('../services/metrics');

const router = express.Router();

// Brute-force / credential-stuffing defence. bcrypt is CPU-bound (10
// rounds ≈ 100 ms), so unrate-limited login is also a DoS vector.
const loginLimiter = rateLimit({
  windowMs: 15 * 60 * 1000,
  max: 5,
  standardHeaders: 'draft-7',
  legacyHeaders: false,
  message: { error: 'Trop de tentatives. Réessaie dans 15 minutes.' },
});

const registerLimiter = rateLimit({
  windowMs: 60 * 60 * 1000,
  max: 10,
  standardHeaders: 'draft-7',
  legacyHeaders: false,
  message: { error: 'Trop de comptes créés depuis cette IP.' },
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
      return res.status(400).json({ error: 'Tous les champs sont requis.' });
    }

    if (username.length < 3 || username.length > 20) {
      return res.status(400).json({ error: 'Le pseudo doit faire entre 3 et 20 caracteres.' });
    }

    if (!/^[a-zA-Z0-9_-]+$/.test(username)) {
      return res.status(400).json({ error: 'Le pseudo ne peut contenir que des lettres, chiffres, - et _.' });
    }

    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      return res.status(400).json({ error: 'Email invalide.' });
    }

    if (password.length < 6) {
      return res.status(400).json({ error: 'Le mot de passe doit faire au moins 6 caracteres.' });
    }

    if (await findUserByEmail(email)) {
      return res.status(400).json({ error: 'Cet email est deja utilise.' });
    }

    if (await findUserByUsername(username)) {
      return res.status(400).json({ error: 'Ce pseudo est deja pris.' });
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
    res.status(500).json({ error: 'Erreur serveur.' });
  }
});

router.post('/login', loginLimiter, async (req, res) => {
  try {
    const { email, password } = req.body;

    if (!email || !password) {
      return res.status(400).json({ error: 'Email et mot de passe requis.' });
    }

    const user = await findUserByEmail(email);
    if (!user) {
      return res.status(401).json({ error: 'Email ou mot de passe incorrect.' });
    }

    const valid = await bcrypt.compare(password, user.password_hash);
    if (!valid) {
      return res.status(401).json({ error: 'Email ou mot de passe incorrect.' });
    }

    const token = jwt.sign({ userId: user.id, username: user.username }, config.JWT_SECRET, { expiresIn: '7d' });

    res.cookie('token', token, COOKIE_OPTIONS);
    metrics.authAttemptsCounter.inc({ kind: 'login', status: 'ok' });
    res.json({ user: { id: user.id, username: user.username, color: user.color } });
  } catch (err) {
    log.error({ err: err.message }, 'login error');
    metrics.authAttemptsCounter.inc({ kind: 'login', status: 'error' });
    res.status(500).json({ error: 'Erreur serveur.' });
  }
});

router.post('/logout', (req, res) => {
  res.clearCookie('token', COOKIE_OPTIONS);
  res.json({ ok: true });
});

router.get('/me', optionalAuth, (req, res) => {
  if (!req.user) {
    return res.status(401).json({ error: 'Not authenticated' });
  }
  res.json({ user: { id: req.user.id, username: req.user.username, color: req.user.color } });
});

module.exports = router;

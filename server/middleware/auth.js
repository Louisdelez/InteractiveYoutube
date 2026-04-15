const jwt = require('jsonwebtoken');
const config = require('../config');
const { findUserById } = require('../db');

async function optionalAuth(req, res, next) {
  req.user = null;
  const token = req.cookies?.token;
  if (token) {
    try {
      const decoded = jwt.verify(token, config.JWT_SECRET);
      req.user = await findUserById(decoded.userId);
    } catch (err) {
      // Invalid token, continue as anonymous
    }
  }
  next();
}

async function requireAuth(req, res, next) {
  await optionalAuth(req, res, () => {
    if (!req.user) {
      return res.status(401).json({ error: 'Authentication required' });
    }
    next();
  });
}

module.exports = { optionalAuth, requireAuth };

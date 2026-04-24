/**
 * i18n lookup for server-emitted messages (mostly Socket.IO error
 * strings). Source of truth is `shared/i18n/fr.json`. Loaded once at
 * boot.
 *
 * Usage:
 *   const { t } = require('../i18n/fr');
 *   t('chat.rate_limit_error');   // => "Trop de messages, attends un peu."
 */
const path = require('path');
const fs = require('fs');

const bundle = JSON.parse(
  fs.readFileSync(path.join(__dirname, '..', '..', 'shared', 'i18n', 'fr.json'), 'utf-8')
);

const warned = new Set();

function t(key) {
  const v = bundle[key];
  if (typeof v === 'string') return v;
  if (!warned.has(key)) {
    warned.add(key);
    // eslint-disable-next-line no-console
    console.warn(`[i18n] missing key: ${key}`);
  }
  return key;
}

module.exports = { t };

// i18n lookup for the web client. Source of truth is
// shared/i18n/fr.json; Vite bundles it at build time via the
// `?url` or direct import (JSON modules are supported natively).
//
// Usage:
//   import { t } from '../i18n';
//   t('chat.title')   // => "Chat en direct"
//
// Missing keys return the key itself as a best-effort fallback so
// a translation gap is visible but non-blocking.

import fr from '../../../shared/i18n/fr.json';

const bundle = fr;

export function t(key, params) {
  const value = bundle[key];
  if (typeof value !== 'string') {
    // Key not found — log once per key to aid detection.
    if (!t._warned) t._warned = new Set();
    if (!t._warned.has(key)) {
      t._warned.add(key);
      // eslint-disable-next-line no-console
      console.warn(`[i18n] missing key: ${key}`);
    }
    return key;
  }
  if (!params) return value;
  // Interpolate `{name}` placeholders. Missing keys in `params` are
  // left in the output as-is (visible in UI = easier to notice).
  return value.replace(/\{(\w+)\}/g, (m, k) =>
    Object.prototype.hasOwnProperty.call(params, k) ? String(params[k]) : m,
  );
}

export default { t };

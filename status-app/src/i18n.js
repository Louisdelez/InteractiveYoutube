// i18n lookup for the standalone status app. Shares the canonical
// bundle with the main web client (shared/i18n/fr.json) via Vite's
// JSON module import — no duplication.

import fr from '../../shared/i18n/fr.json';

const bundle = fr;

export function t(key, params) {
  const value = bundle[key];
  if (typeof value !== 'string') {
    if (!t._warned) t._warned = new Set();
    if (!t._warned.has(key)) {
      t._warned.add(key);
      // eslint-disable-next-line no-console
      console.warn(`[i18n] missing key: ${key}`);
    }
    return key;
  }
  if (!params) return value;
  return value.replace(/\{(\w+)\}/g, (m, k) =>
    Object.prototype.hasOwnProperty.call(params, k) ? String(params[k]) : m,
  );
}

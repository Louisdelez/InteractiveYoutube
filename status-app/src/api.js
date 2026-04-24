const BASE = import.meta.env.VITE_API_URL || 'http://localhost:4500';

export async function get(path) {
  const res = await fetch(BASE + path, { credentials: 'omit' });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    throw new Error(text || `HTTP ${res.status}`);
  }
  return res.json();
}

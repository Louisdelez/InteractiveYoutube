# Koala TV Status Page

Standalone status page for Koala TV. Deployable on its own subdomain
(e.g. `status.koalatv.com`), completely decoupled from the main web
client — only reads from the Koala TV API.

## Dev

```bash
cd status-app
npm install
cp .env.example .env     # adjust VITE_API_URL if your server isn't on :4500
npm run dev              # http://localhost:4502
```

The page polls `VITE_API_URL/api/status`, `/api/status/history`,
`/api/status/incidents` every 30 s.

## Production build

```bash
VITE_API_URL=https://api.koalatv.com npm run build
# dist/ contains the static bundle — drop it on any static host
```

Deploy targets that work out of the box: nginx, Cloudflare Pages,
Netlify, GitHub Pages, Vercel, S3+CloudFront.

### Example nginx snippet

```nginx
server {
  listen 443 ssl http2;
  server_name status.koalatv.com;

  ssl_certificate     /etc/letsencrypt/live/status.koalatv.com/fullchain.pem;
  ssl_certificate_key /etc/letsencrypt/live/status.koalatv.com/privkey.pem;

  root /var/www/status;
  index index.html;

  location / {
    try_files $uri $uri/ /index.html;
  }
}
```

## API contract

The status page expects these endpoints on `VITE_API_URL`:

- `GET /api/status` → `{ components: [...], summary: { overall }, ts }`
- `GET /api/status/history?days=90` → `{ days, history: { [componentId]: [{ day, status }] } }`
- `GET /api/status/incidents` → `{ incidents: [...] }`

These are served by `server/routes/status.js` in the main Koala TV
backend. CORS is opened to `*` on that route so the status page can
live on any subdomain.

## Why a separate app?

- Survives a Koala TV outage: the status page can be hosted on a
  different host, CDN or provider, and still render "API server: DOWN"
  while the main site is unreachable.
- Independent deploy cadence — status HTML/CSS/JS rarely changes.
- No SSR / auth / cookies needed: pure static files + CORS.

## Branding

- Logo: drop `koala-tv.png` into `public/` (same image as the main
  site favicon).
- Colors mirror the main client's dark theme.

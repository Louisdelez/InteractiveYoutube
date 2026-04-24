import { defineConfig, loadEnv } from 'vite';
import react from '@vitejs/plugin-react';

// Standalone deployment:
//   - dev:     VITE_API_URL defaults to http://localhost:4500
//   - prod:    build with `VITE_API_URL=https://api.koalatv.com npm run build`
//     then serve the `dist/` folder from any static host (nginx, Cloudflare
//     Pages, Netlify, GitHub Pages…).
//
// Dev port 4502 to avoid clashing with the main client (4501).

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '');
  return {
    plugins: [react()],
    server: {
      port: 4502,
      host: '0.0.0.0',
    },
    define: {
      'import.meta.env.VITE_API_URL': JSON.stringify(env.VITE_API_URL || 'http://localhost:4500'),
    },
  };
});

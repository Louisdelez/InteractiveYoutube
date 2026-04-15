import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 4501,
    proxy: {
      '/api': 'http://localhost:4500',
      '/socket.io': {
        target: 'http://localhost:4500',
        ws: true,
      },
    },
  },
})

import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// kou-router backend (axum) — default bind, override with KOU_ROUTER_BIND
const backend = process.env.KOU_BACKEND ?? 'http://127.0.0.1:20128'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    allowedHosts: ['4be6-72-56-70-209.ngrok-free.app'],
    proxy: {
      '/api': backend,
      '/v1': backend,
      '/health': backend,
    },
  },
})

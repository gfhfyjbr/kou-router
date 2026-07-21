import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// kou-router backend (axum) — default bind, override with KOU_ROUTER_BIND
const backend = process.env.KOU_BACKEND ?? 'http://127.0.0.1:20128'
const root = path.dirname(fileURLToPath(import.meta.url))
const uiKitRoot = path.resolve(root, 'vendor/kou-ui-kit')
const uiKitSrc = path.resolve(uiKitRoot, 'src/index.ts')
const uiKitStyles = path.resolve(uiKitRoot, 'src/styles/kou.css')

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  resolve: {
    dedupe: ['react', 'react-dom'],
    alias: [
      { find: '@kou/ui-kit/styles.css', replacement: uiKitStyles },
      { find: /^@kou\/ui-kit$/, replacement: uiKitSrc },
    ],
  },
  server: {
    allowedHosts: ['4be6-72-56-70-209.ngrok-free.app'],
    proxy: {
      '/api': backend,
      '/v1': backend,
      '/health': backend,
    },
  },
})

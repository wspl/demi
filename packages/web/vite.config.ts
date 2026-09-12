import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  // The repository's `.env` carries the local development account
  // (`DEMI_DEV_EMAIL`, `DEMI_DEV_PASSWORD`); the sign-in page fills it in
  // during development only.
  envDir: resolve(import.meta.dirname, '../..'),
  envPrefix: ['VITE_', 'DEMI_DEV_'],
  resolve: {
    alias: {
      '@': resolve(import.meta.dirname, 'src'),
      '@demicodes/web-ui': resolve(import.meta.dirname, '../web-ui/src'),
    },
  },
  server: {
    host: '127.0.0.1',
    port: 18934,
    strictPort: true,
    proxy: {
      '/api': {
        target: process.env.DEMI_BACKEND_URL ?? 'http://127.0.0.1:3271',
        ws: true,
      },
    },
  },
})

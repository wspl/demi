import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
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

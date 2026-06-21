import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig(() => ({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      '@': resolve(import.meta.dirname, 'src/app'),
      '@demi/web-ui': resolve(import.meta.dirname, '../web-ui/src'),
    },
  },
  server: {
    port: Number(process.env.PORT) || 5173,
    proxy: {
      '/agent': { target: 'ws://localhost:8787', ws: true },
      '/control': { target: 'ws://localhost:8787', ws: true },
    },
  },
}))

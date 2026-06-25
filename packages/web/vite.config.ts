import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { WEB_DEV_HOST, WEB_FRONTEND_PORT } from './src/dev-ports'

export default defineConfig(() => ({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      '@': resolve(import.meta.dirname, 'src/app'),
      '@demi/web-ui': resolve(import.meta.dirname, '../web-ui/src'),
    },
  },
  server: {
    host: WEB_DEV_HOST,
    port: WEB_FRONTEND_PORT,
    strictPort: true,
  },
}))

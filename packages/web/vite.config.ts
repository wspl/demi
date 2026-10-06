import { randomUUID } from 'node:crypto'
import { resolve } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

/**
 * Names each build of the web app (`web-application.md` § A page of
 * another build): the page carries the id as `import.meta.env.DEMI_WEB_BUILD`,
 * and `build.json` beside it tells the backend that serves it. Vite's
 * development server serves sources, which name no build.
 */
function webBuild(): Plugin {
  const build = randomUUID()
  return {
    name: 'demi-web-build',
    apply: 'build',
    config: () => ({ define: { 'import.meta.env.DEMI_WEB_BUILD': JSON.stringify(build) } }),
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'build.json', source: JSON.stringify({ build }) })
    },
  }
}

/**
 * The direct channel's service worker (`direct-channel.md` § Bytes the
 * browser fetches itself), served from the root so that its scope is the
 * whole origin: the development server transforms it on request, and the
 * build emits it beside the page under its fixed name.
 */
function directServiceWorker(): Plugin {
  const source = resolve(import.meta.dirname, 'src/direct/service-worker.ts')
  const url = '/direct-sw.js'
  let building = false
  return {
    name: 'demi-direct-service-worker',
    configResolved(config) {
      building = config.command === 'build'
    },
    configureServer(server) {
      server.middlewares.use(url, (_request, response, next) => {
        server
          .transformRequest('/src/direct/service-worker.ts')
          .then((result) => {
            if (!result) {
              next()
              return
            }
            response.setHeader('content-type', 'text/javascript')
            response.setHeader('cache-control', 'no-cache')
            response.end(result.code)
          })
          .catch(next)
      })
    },
    buildStart() {
      if (!building)
        return
      this.emitFile({ type: 'chunk', id: source, fileName: url.slice(1) })
    },
  }
}

export default defineConfig({
  plugins: [vue(), tailwindcss(), webBuild(), directServiceWorker()],
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
      // The proxy keeps the page's Host, which the page's Origin names: that
      // is how the backend knows this page for the product's and lets it act
      // (backend.md § Authentication and ownership).
      '/api': {
        target: process.env.DEMI_BACKEND_URL ?? 'http://127.0.0.1:3271',
        ws: true,
      },
    },
  },
})

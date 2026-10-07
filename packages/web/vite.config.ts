import { randomUUID } from 'node:crypto'
import { readFileSync, readdirSync } from 'node:fs'
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

/**
 * The web preview's page runtime (`builds-and-releases.md` § Preview
 * runtime), which `bun xtask preview-runtime` builds into
 * `@demicodes/preview-runtime`'s `dist/runtime/<release>.js`: the page
 * carries its release as `import.meta.env.DEMI_PREVIEW_RUNTIME`, and the
 * relay delivers it from `/runtime/<release>.js`, which the development
 * server serves from there and the build emits. Without a runtime built,
 * the page carries none and offers no preview.
 */
function previewRuntime(): Plugin {
  const directory = resolve(import.meta.dirname, '../preview-runtime/dist/runtime')
  let file: string | undefined
  try {
    file = readdirSync(directory).find((name) => name.endsWith('.js'))
  } catch {
    // No runtime was built: the page offers no preview.
  }
  const release = file?.slice(0, -'.js'.length) ?? ''
  return {
    name: 'demi-preview-runtime',
    config: () => ({ define: { 'import.meta.env.DEMI_PREVIEW_RUNTIME': JSON.stringify(release) } }),
    configureServer(server) {
      if (!file) {
        return
      }
      const path = resolve(directory, file)
      server.middlewares.use(`/runtime/${file}`, (_request, response) => {
        response.setHeader('content-type', 'text/javascript')
        response.setHeader('cache-control', 'no-cache')
        response.end(readFileSync(path))
      })
    },
    generateBundle() {
      if (file) {
        this.emitFile({ type: 'asset', fileName: `runtime/${file}`, source: readFileSync(resolve(directory, file)) })
      }
    },
  }
}

/**
 * Keeps the page's hot-update socket on the development server itself,
 * even for a page loaded through a forwarder in front of it, as `bun browse`
 * loads it (`browse.md` § The slot's servers): Vite reloads a page whose
 * hot-update socket closed once the server answers again, so a restart of
 * the forwarder would otherwise reload the page and lose what was typed in
 * it. The port is the one the server listens on, `--port` included.
 */
function directHotUpdates(): Plugin {
  return {
    name: 'demi-direct-hot-updates',
    apply: 'serve',
    config: (config) => ({ server: { hmr: { clientPort: config.server?.port } } }),
  }
}

export default defineConfig({
  plugins: [vue(), tailwindcss(), webBuild(), directServiceWorker(), directHotUpdates(), previewRuntime()],
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

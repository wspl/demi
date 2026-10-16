import { randomUUID } from 'node:crypto'
import { readFileSync } from 'node:fs'
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
    // The app's page names its build too, so the service worker keeps it
    // under that build (`web-application.md` § Installing and opening offline).
    transformIndexHtml: (html, context) =>
      context.filename.endsWith('/index.html')
        ? html.replace('</head>', `  <meta name="demi-build" content="${build}" />\n  </head>`)
        : html,
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'build.json', source: JSON.stringify({ build }) })
    },
  }
}

/**
 * The Demi version the page shows at Settings › About: the workspace version
 * in `Cargo.toml`, which every program of the same checkout carries
 * (`demi_shared_artifacts::WORKSPACE_VERSION`).
 */
function workspaceVersion(): string {
  const manifest = readFileSync(resolve(import.meta.dirname, '../../Cargo.toml'), 'utf8')
  // The `version` line of the `[workspace.package]` table, as the release
  // workflow reads it.
  const version = /^\[workspace\.package\]$[^[]*?^version = "([^"]+)"$/m.exec(manifest)?.[1]
  if (!version)
    throw new Error('Cargo.toml names no workspace version')
  return version
}

/**
 * The page's one service worker (`direct-channel.md` § Bytes the browser
 * fetches itself, `web-application.md` § Installing and opening offline),
 * served from the root so that its scope is the whole origin: the
 * development server transforms it on request, and the build emits it
 * beside the page under its fixed name.
 */
function serviceWorker(): Plugin {
  const source = resolve(import.meta.dirname, 'src/offline/service-worker.ts')
  const url = '/service-worker.js'
  let building = false
  return {
    name: 'demi-service-worker',
    configResolved(config) {
      building = config.command === 'build'
    },
    configureServer(server) {
      server.middlewares.use(url, (_request, response, next) => {
        server
          .transformRequest('/src/offline/service-worker.ts')
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

/**
 * The shared page's own entry (`sharing.md` § Safety): the build emits
 * `share.html` beside `index.html`, which the backend serves at
 * `/s/:token`; the development server answers that address with it the
 * same way.
 */
function sharedPage(): Plugin {
  return {
    name: 'demi-shared-page',
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use((request, _response, next) => {
        if (request.url && /^\/s\/[^/?#]+\/?(?:[?#]|$)/.test(request.url)) {
          request.url = '/share.html'
        }
        next()
      })
    },
  }
}

export default defineConfig({
  plugins: [vue(), tailwindcss(), webBuild(), serviceWorker(), directHotUpdates(), sharedPage()],
  // The repository's `.env` carries the local development account
  // (`DEMI_DEV_EMAIL`, `DEMI_DEV_PASSWORD`); the sign-in page fills it in
  // during development only.
  envDir: resolve(import.meta.dirname, '../..'),
  envPrefix: ['VITE_', 'DEMI_DEV_'],
  define: { 'import.meta.env.DEMI_VERSION': JSON.stringify(workspaceVersion()) },
  build: {
    // The npm packages the bundle holds, each with its license and the
    // text of its license file, from the bundle's own module graph:
    // `xtask notices` reads it (`builds-and-releases.md` § Licenses in a
    // release).
    license: { fileName: '.vite/license.json' },
    rollupOptions: {
      input: {
        index: resolve(import.meta.dirname, 'index.html'),
        share: resolve(import.meta.dirname, 'share.html'),
      },
    },
  },
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

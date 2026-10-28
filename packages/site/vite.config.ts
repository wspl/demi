import { readFileSync } from 'node:fs'
import { extname, resolve } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { ICONS } from './src/icons'

// The site's pages, or, in the `record` mode `record.ts` builds in, the demo
// app it records.
export default defineConfig(({ mode }) => ({
  plugins: [vue(), tailwindcss(), icons(), seo()],
  resolve: {
    alias: {
      '@': resolve(import.meta.dirname, 'src'),
      '@demicodes/web-ui': resolve(import.meta.dirname, '../web-ui/src'),
      '@demicodes/web-gallery': resolve(import.meta.dirname, '../web-gallery/src'),
    },
  },
  build: {
    rollupOptions: {
      input: mode === 'record'
        ? [resolve(import.meta.dirname, 'record.html')]
        : ['index.html', 'demo.html', 'og.html'].map((page) => resolve(import.meta.dirname, page)),
    },
  },
  server: {
    host: '127.0.0.1',
    port: 18934,
    strictPort: true,
  },
}))

/** The app's icons, from their one source in `web` and `web-ui`, at the site's root. */
function icons(): Plugin {
  const web = (path: string): string => resolve(import.meta.dirname, '..', path)
  const sources: Record<string, string> = {
    [ICONS.favicon]: web('web/public/icons/favicon.svg'),
    [ICONS.appleTouch]: web('web-ui/src/assets/app-icon.png'),
    [ICONS.small]: web('web/public/icons/icon-192.png'),
    [ICONS.large]: web('web/public/icons/icon-512.png'),
  }
  return {
    name: 'demi-site-icons',
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const source = sources[request.url?.slice(1).split('?')[0] ?? '']
        if (!source)
          return next()
        response.setHeader('content-type', TYPES[extname(source)] ?? 'application/octet-stream')
        response.end(readFileSync(source))
      })
    },
    generateBundle() {
      for (const [fileName, source] of Object.entries(sources))
        this.emitFile({ type: 'asset', fileName, source: readFileSync(source) })
    },
  }
}

/**
 * The page's head tags and the files beside it (`seo.ts`), on the
 * development server; `build.ts` writes the same into the build.
 */
function seo(): Plugin {
  // What `src/entry-server.ts` exports for it.
  type Seo = { headHtml: () => string, textFiles: () => Record<string, string> }
  return {
    name: 'demi-site-seo',
    apply: 'serve',
    configureServer(server) {
      const load = () => server.ssrLoadModule('/src/entry-server.ts') as Promise<Seo>
      server.middlewares.use(async (request, response, next) => {
        const name = request.url?.slice(1).split('?')[0] ?? ''
        const text = (await load()).textFiles()[name]
        if (text === undefined)
          return next()
        response.setHeader('content-type', `${TYPES[extname(name)] ?? 'text/plain'}; charset=utf-8`)
        response.end(text)
      })
    },
    async transformIndexHtml(html, context) {
      if (!html.includes('<!--head-->'))
        return html
      const module = (await context.server!.ssrLoadModule('/src/entry-server.ts')) as Seo
      return html.replace('<!--head-->', module.headHtml())
    },
  }
}

const TYPES: Record<string, string> = {
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.txt': 'text/plain',
  '.xml': 'application/xml',
  '.webmanifest': 'application/manifest+json',
}

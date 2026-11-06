import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

/** The widget plugin's files, which the backend serves from its binary. */
const WIDGET_FILES = resolve(import.meta.dirname, '../../crates/plugin-widget/plugin/vendor')

/**
 * The backend's widget routes a gallery widget's frame reaches, answered as
 * the backend answers them: a widget's document, from the generated
 * specimens, and the plugin's files its document loads by a root-relative
 * address, which a blob URL's document could not resolve. Only the
 * development server answers them: the gallery is never built to be served.
 */
function widgetRoutes(): Plugin {
  return {
    name: 'gallery-widget-routes',
    configureServer(server) {
      server.middlewares.use(async (request, response, next) => {
        const path = request.url?.split('?')[0] ?? ''
        const file = /^\/api\/plugins\/widget\/files\/([\w-]+(?:\.[\w-]+)*\.js)$/.exec(path)
        const block = /^\/api\/plugins\/widget\/blocks\/widget\/(\w+)$/.exec(path)
        if (!file && !block)
          return next()
        const answer = file
          ? await readFile(resolve(WIDGET_FILES, file[1])).then(
            (bytes) => ({ type: 'text/javascript; charset=utf-8', bytes }),
            () => undefined,
          )
          : await server.ssrLoadModule('/src/generated/widgets.ts').then(({ GALLERY_WIDGETS }) => {
            const document: string | undefined = GALLERY_WIDGETS[block![1]]?.document
            return document === undefined ? undefined : { type: 'text/html; charset=utf-8', bytes: document }
          })
        if (!answer) {
          response.statusCode = 404
          return response.end()
        }
        response.setHeader('Content-Type', answer.type)
        response.setHeader('Access-Control-Allow-Origin', '*')
        response.end(answer.bytes)
      })
    },
  }
}

export default defineConfig({
  plugins: [vue(), tailwindcss(), widgetRoutes()],
  resolve: {
    alias: {
      '@': resolve(import.meta.dirname, 'src'),
      '@demicodes/web-ui': resolve(import.meta.dirname, '../web-ui/src'),
    },
  },
  server: {
    device: '127.0.0.1',
    port: 18933,
    strictPort: true,
  },
})

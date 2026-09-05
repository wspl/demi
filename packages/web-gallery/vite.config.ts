import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { defineConfig, type Plugin } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

/** Persists Color review decisions to a file the reviewer's collaborator can read back. */
function colorReviewPlugin(): Plugin {
  const file = resolve(import.meta.dirname, '.color-review/decisions.json')
  return {
    name: 'demi-color-review',
    configureServer(server) {
      server.middlewares.use('/__color-review/decisions', async (req, res) => {
        res.setHeader('content-type', 'application/json')
        if (req.method === 'GET') {
          try {
            res.end(await readFile(file, 'utf8'))
          } catch {
            res.end('{}')
          }
          return
        }
        if (req.method === 'PUT') {
          let raw = ''
          for await (const chunk of req) raw += chunk
          let parsed: unknown
          try {
            parsed = JSON.parse(raw)
          } catch {
            res.statusCode = 400
            res.end('{}')
            return
          }
          await mkdir(dirname(file), { recursive: true })
          await writeFile(file, `${JSON.stringify(parsed, null, 2)}\n`)
          res.statusCode = 204
          res.end()
          return
        }
        res.statusCode = 405
        res.end('{}')
      })
    },
  }
}

export default defineConfig({
  plugins: [vue(), tailwindcss(), colorReviewPlugin()],
  resolve: {
    alias: {
      '@': resolve(import.meta.dirname, 'src'),
      '@demicodes/web-ui': resolve(import.meta.dirname, '../web-ui/src'),
    },
  },
  server: {
    host: '127.0.0.1',
    port: 18933,
    strictPort: true,
  },
})

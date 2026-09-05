// The web app's dev entry: Vite alone. The app talks to a running
// `@demicodes/backend` (`docs/demi-next/backend.md`); the browser
// application itself is rebuilt on that API in M13.
import process from 'node:process'
import { resolve } from 'node:path'
import { WEB_DEV_HOST, WEB_FRONTEND_PORT, WEB_FRONTEND_URL } from './dev-ports'

const packageRoot = resolve(import.meta.dirname, '..')
const vite = Bun.spawn([process.execPath, 'x', 'vite', '--host', WEB_DEV_HOST, '--port', String(WEB_FRONTEND_PORT), '--strictPort'], {
  cwd: packageRoot,
  stdin: 'inherit',
  stdout: 'inherit',
  stderr: 'inherit',
  env: process.env,
})
process.stdout.write(`demi web frontend: ${WEB_FRONTEND_URL}\n`)
process.on('SIGINT', () => vite.kill('SIGTERM'))
process.on('SIGTERM', () => vite.kill('SIGTERM'))
process.exit(await vite.exited)

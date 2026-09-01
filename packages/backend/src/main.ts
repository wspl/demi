import { homedir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { createBackend } from './backend'

async function main(): Promise<void> {
  const dataDir = process.env.DEMI_BACKEND_DATA ?? join(homedir(), '.demi', 'backend')
  const port = Number(process.env.DEMI_BACKEND_PORT ?? 3271)
  const backend = await createBackend({ dataDir, port })
  console.log(`demi-backend listening on ${backend.url} (data: ${dataDir})`)
  console.log('Providers come from connections: add one via POST /api/connections (or the web UI).')

  const shutdown = () => {
    void backend.close().then(() => process.exit(0))
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}

void main()

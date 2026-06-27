import process from 'node:process'
import { resolve } from 'node:path'
import { WEB_BACKEND_BASE_URL, WEB_BACKEND_PORT, WEB_FRONTEND_PORT, WEB_FRONTEND_URL, WEB_DEV_HOST } from './dev-ports'
import { killListeningPorts } from './server/port-cleanup'

const packageRoot = resolve(import.meta.dirname, '..')
const childProcesses: Bun.Subprocess[] = []
let shuttingDown = false

await killListeningPorts([WEB_BACKEND_PORT, WEB_FRONTEND_PORT])

start('backend', [process.execPath, 'run', 'src/server/index.ts'])
start('frontend', [
  process.execPath,
  'x',
  'vite',
  '--host',
  WEB_DEV_HOST,
  '--port',
  String(WEB_FRONTEND_PORT),
  '--strictPort',
])

process.stdout.write(`demi web backend: ${WEB_BACKEND_BASE_URL}\n`)
process.stdout.write(`demi web frontend: ${WEB_FRONTEND_URL}\n`)

process.on('SIGINT', () => void shutdown(0))
process.on('SIGTERM', () => void shutdown(0))

await new Promise(() => {})

function start(name: string, cmd: string[]): void {
  const child = Bun.spawn(cmd, {
    cwd: packageRoot,
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
    env: process.env,
  })
  childProcesses.push(child)
  void child.exited.then((exitCode) => {
    if (shuttingDown) return
    const code = exitCode ?? 1
    process.stderr.write(`${name} exited with code ${code}\n`)
    void shutdown(code === 0 ? 0 : code)
  })
}

async function shutdown(code: number): Promise<void> {
  if (shuttingDown) return
  shuttingDown = true
  for (const child of childProcesses) child.kill()
  await Promise.allSettled(childProcesses.map((child) => child.exited))
  process.exit(code)
}

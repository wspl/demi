import { join } from 'node:path'
import { expect, test } from 'bun:test'

// The production entry point reads its whole environment before it opens
// anything (`managed-hosts-setup.md` § Configuration). An unusable value stops
// the process there, naming the variable, rather than turning into a NaN port
// or an unset instance mode later on.

const REPO_ROOT = join(import.meta.dir, '..', '..', '..', '..')

/** The process, started with exactly these `DEMI_*` variables. */
function startBackend(demi: Record<string, string>) {
  const env: Record<string, string | undefined> = { ...process.env }
  for (const name of Object.keys(env)) {
    if (name.startsWith('DEMI_'))
      delete env[name]
  }
  return Bun.spawnSync({
    cmd: [
      'bun',
      'run',
      '--conditions',
      'development',
      'packages/backend/src/main.ts',
    ],
    cwd: REPO_ROOT,
    env: { ...env, ...demi },
    stdout: 'pipe',
    stderr: 'pipe',
  })
}

test('a port that is not a number stops startup and names the variable', () => {
  const started = startBackend({
    DEMI_BACKEND_PORT: 'abc',
    DEMI_INSTANCE_MODE: 'shared',
    DEMI_NATIVE_CONFIG: '/nonexistent/native.json',
  })
  expect(started.exitCode).not.toBe(0)
  expect(started.stderr.toString()).toContain('DEMI_BACKEND_PORT')
})

test('managed hosts without the URL guests dial stops startup', () => {
  const started = startBackend({
    DEMI_INSTANCE_MODE: 'shared',
    DEMI_NATIVE_CONFIG: '/nonexistent/native.json',
    DEMI_MACHINES_SOCKET: '/tmp/demi-machines.sock',
  })
  expect(started.exitCode).not.toBe(0)
  expect(started.stderr.toString()).toContain('DEMI_BACKEND_PUBLIC_URL')
})

import { randomUUID } from 'node:crypto'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'

/**
 * Demi local state root (not the workspace cwd).
 * Override with `DEMI_HOME` or `createLocalAgentServer({ stateDir })`, else `~/.demi`.
 */
export function resolveDemiHome(explicit?: string): string {
  if (explicit && explicit.trim()) return resolve(explicit.trim())
  const fromEnv = process.env.DEMI_HOME
  if (fromEnv && fromEnv.trim()) return resolve(fromEnv.trim())
  return join(homedir(), '.demi')
}

/** Fixed layout: `<stateDir>/bridges/<id>.sock` (short id for macOS AF_UNIX limits). */
export function defaultBridgeSocketPath(stateDir: string, serverId = randomUUID().replace(/-/g, '').slice(0, 12)): string {
  return join(stateDir, 'bridges', `${serverId}.sock`)
}

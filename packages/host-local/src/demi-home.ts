import { randomUUID } from 'node:crypto'
import { join } from 'node:path'
export { resolveDemiHome } from '@demicodes/provider/credentials-pool'

/** Fixed layout: `<stateDir>/bridges/<id>.sock` (short id for macOS AF_UNIX limits). */
export function defaultBridgeSocketPath(stateDir: string, serverId = randomUUID().replace(/-/g, '').slice(0, 12)): string {
  return join(stateDir, 'bridges', `${serverId}.sock`)
}

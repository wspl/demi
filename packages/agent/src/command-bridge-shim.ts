import { encodeUtf8 } from '@demicodes/utils'
import type { Host } from '@demicodes/shell'

const SHIM_ROOT = '.demi-bin'
const DISPATCH_FILE = '.dispatch'
const DISPATCH_PACKAGE_JSON = '{"type":"commonjs"}\n'

/**
 * Rejects an `agentSessionId` that cannot be used as one filesystem path
 * segment. `agentSessionId` is client-supplied and becomes a real path
 * component under Host.fs, so it is validated here rather than trusted.
 */
function assertPathSafeSessionId(agentSessionId: string): void {
  if (agentSessionId.length === 0 || agentSessionId.includes('/') || agentSessionId.includes('\\') || agentSessionId === '..') {
    throw new Error(`Command bridge: agentSessionId "${agentSessionId}" is not safe to use as a path segment`)
  }
}

/**
 * Writes the command bridge shim directory for one session and returns its
 * resolved absolute path for PATH injection. Idempotent: safe on every open().
 */
export async function materializeCommandBridgeShims(
  host: Host,
  agentSessionId: string,
  commandNames: readonly string[],
  shimSource: string,
): Promise<string> {
  assertPathSafeSessionId(agentSessionId)
  const dir = `${SHIM_ROOT}/${agentSessionId}`
  await host.fs.mkdir(dir, { recursive: true })
  await host.fs.writeFile(`${dir}/package.json`, encodeUtf8(DISPATCH_PACKAGE_JSON))
  await host.fs.writeFile(`${dir}/${DISPATCH_FILE}`, encodeUtf8(shimSource))
  await host.fs.chmod(`${dir}/${DISPATCH_FILE}`, 0o755)

  const wanted = new Set(commandNames)
  const existing = await host.fs.readdir(dir)
  for (const entry of existing) {
    if (entry === DISPATCH_FILE || entry === 'package.json' || wanted.has(entry)) continue
    await host.fs.rm(`${dir}/${entry}`, { force: true })
  }
  for (const name of wanted) {
    const linkPath = `${dir}/${name}`
    await host.fs.rm(linkPath, { force: true })
    await host.fs.symlink(DISPATCH_FILE, linkPath)
  }
  return host.fs.realpath(dir)
}

import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'
import { nativePackageSchema, NATIVE_TARGETS, type ArtifactResolver } from '@demicodes/command-protocol'
import info from '../package-info.json'

let fixture: ReturnType<typeof buildFixture> | undefined

/** Host-only test executable and catalog. Never publish this fixture descriptor. */
export function nativePackageFixture() {
  return fixture ??= buildFixture()
}

async function buildFixture() {
  const root = resolve(import.meta.dir, '../../..')
  const executable = process.env.DEMI_NATIVE_TEST_BINARY ?? join(root, 'target/debug', `demi-commands${process.platform === 'win32' ? '.exe' : ''}`)
  if (!process.env.DEMI_NATIVE_TEST_BINARY) {
    const installed = join(homedir(), '.cargo/bin', process.platform === 'win32' ? 'cargo.exe' : 'cargo')
    const child = Bun.spawn([existsSync(installed) ? installed : 'cargo', 'build', '-p', 'demi-builtin-package'], {
      cwd: root, stdout: 'ignore', stderr: 'pipe',
    })
    const diagnostics = new Response(child.stderr).text()
    if (await child.exited !== 0)
      throw new Error(`Native fixture build failed: ${await diagnostics}`)
    await diagnostics
  }
  const bytes = await readFile(executable)
  const artifact = { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length }
  // The schema requires all target slots. Tests launch only the current host.
  const descriptor = nativePackageSchema.parse({ ...info, version: `${info.version}-test`,
    targets: Object.fromEntries(NATIVE_TARGETS.map(target => [target, artifact])) })
  const resolveArtifact: ArtifactResolver = async (requested, signal) => {
    signal.throwIfAborted()
    if (requested.sha256 !== artifact.sha256 || requested.size !== artifact.size)
      throw new Error('Artifact is not in the host-only test catalog')
    return { path: executable }
  }
  return { packages: [descriptor], resolveArtifact }
}

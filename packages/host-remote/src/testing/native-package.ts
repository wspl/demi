import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { nativePackageSchema, NATIVE_TARGETS, type ArtifactResolver } from '@demicodes/command-protocol'
import {
  NATIVE_PACKAGE_CRATES,
  readPackageInfo,
  type NativePackageCrate,
} from '../../../../scripts/native/package-info'

/**
 * Host-only test executables and catalog, one package per command crate. Never
 * publish these fixture descriptors. `DEMI_NATIVE_TEST_BINARY` names a prebuilt
 * `demi-commands`; the other crates are always built here.
 */
export async function buildNativePackageFixture(root: string) {
  const prebuilt: Partial<Record<NativePackageCrate, string>> = process.env.DEMI_NATIVE_TEST_BINARY
    ? { 'demi-commands': process.env.DEMI_NATIVE_TEST_BINARY }
    : {}
  const building = NATIVE_PACKAGE_CRATES.filter(crate => !prebuilt[crate])
  if (building.length > 0) {
    const installed = join(homedir(), '.cargo/bin', process.platform === 'win32' ? 'cargo.exe' : 'cargo')
    const child = Bun.spawn([
      existsSync(installed) ? installed : 'cargo', 'build',
      ...building.flatMap(crate => ['-p', crate]),
    ], { cwd: root, stdout: 'ignore', stderr: 'pipe' })
    const diagnostics = new Response(child.stderr).text()
    if (await child.exited !== 0)
      throw new Error(`Native fixture build failed: ${await diagnostics}`)
    await diagnostics
  }
  const executables = new Map<string, string>()
  const packages = await Promise.all(NATIVE_PACKAGE_CRATES.map(async (crate) => {
    const info = await readPackageInfo(crate)
    const executable = prebuilt[crate]
      ?? join(root, 'target/debug', `${crate}${process.platform === 'win32' ? '.exe' : ''}`)
    const bytes = await readFile(executable)
    const artifact = { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length }
    executables.set(`${artifact.sha256}:${artifact.size}`, executable)
    // The schema requires all target slots. Tests launch only the current host.
    return nativePackageSchema.parse({ ...info, version: `${info.version}-test`,
      targets: Object.fromEntries(NATIVE_TARGETS.map(target => [target, artifact])) })
  }))
  const resolveArtifact: ArtifactResolver = async (requested, signal) => {
    signal.throwIfAborted()
    const path = executables.get(`${requested.sha256}:${requested.size}`)
    if (!path)
      throw new Error('Artifact is not in the host-only test catalog')
    return { path }
  }
  return { packages, resolveArtifact }
}

import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { nativePackageSchema, NATIVE_TARGETS, type ArtifactResolver } from '@demicodes/command-protocol'
import {
  NATIVE_PACKAGE_CRATES,
  readPackageInfo,
  type NativePackageCrate,
} from '../../../../scripts/native/package-info'

/**
 * Host-only test executables and catalog, one package per command crate. Never
 * publish these fixture descriptors. The executables come from `programs`, the
 * directory the test script builds; `DEMI_NATIVE_TEST_BINARY` names another
 * `demi-commands`.
 */
export async function buildNativePackageFixture(programs: string) {
  const prebuilt: Partial<Record<NativePackageCrate, string>> = process.env.DEMI_NATIVE_TEST_BINARY
    ? { 'demi-commands': process.env.DEMI_NATIVE_TEST_BINARY }
    : {}
  const executables = new Map<string, string>()
  const packages = await Promise.all(NATIVE_PACKAGE_CRATES.map(async (crate) => {
    const info = await readPackageInfo(crate)
    const executable = prebuilt[crate]
      ?? join(programs, `${crate}${process.platform === 'win32' ? '.exe' : ''}`)
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

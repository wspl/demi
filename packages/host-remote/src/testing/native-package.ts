import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import { browserOperations } from '@demicodes/browser-protocol'
import {
  nativePackageSchema,
  NATIVE_PROTOCOL_VERSION,
  NATIVE_TARGETS,
  type ArtifactResolver,
} from '@demicodes/command-protocol'

/**
 * The command crates' packages and the operations each serves. The Rust
 * contract crates declare them (builtin-protocol's and claude-protocol's
 * `Operation`), and `cargo xtask native package` writes the release
 * descriptors from there; this TypeScript copy serves the TypeScript tests
 * until they leave with the TypeScript backend.
 */
const NATIVE_PACKAGES = [
  {
    crate: 'demi-commands',
    id: 'demi.builtin',
    operations: [
      'file.read',
      'file.create',
      'file.edit',
      'file.patch',
      'browser.live',
      ...Object.keys(browserOperations).map(name => `browser.${name}`),
    ],
  },
  { crate: 'demi-claude', id: 'demi.claude', operations: ['claude.ensure', 'claude.status'] },
] as const

const workspaceSchema = z.object({ workspace: z.object({ package: z.object({ version: z.string() }) }) })

/**
 * Host-only test executables and catalog, one package per command crate. Never
 * publish these fixture descriptors. The executables come from `programs`, the
 * directory the test script builds; `DEMI_NATIVE_TEST_BINARY` names another
 * `demi-commands`.
 */
export async function buildNativePackageFixture(programs: string) {
  const workspace = workspaceSchema.parse(
    Bun.TOML.parse(await readFile(new URL('../../../../Cargo.toml', import.meta.url), 'utf8')),
  )
  const executables = new Map<string, string>()
  const packages = await Promise.all(NATIVE_PACKAGES.map(async ({ crate, id, operations }) => {
    const prebuilt = crate === 'demi-commands' ? process.env.DEMI_NATIVE_TEST_BINARY : undefined
    const executable = prebuilt ?? join(programs, `${crate}${process.platform === 'win32' ? '.exe' : ''}`)
    const bytes = await readFile(executable)
    const artifact = { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length }
    executables.set(`${artifact.sha256}:${artifact.size}`, executable)
    // The schema requires all target slots. Tests launch only the current host.
    return nativePackageSchema.parse({
      id,
      version: `${workspace.workspace.package.version}-test`,
      protocolVersion: NATIVE_PROTOCOL_VERSION,
      operations,
      targets: Object.fromEntries(NATIVE_TARGETS.map(target => [target, artifact])),
    })
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

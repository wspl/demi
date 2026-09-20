import { readFile } from 'node:fs/promises'
import { z } from 'zod'
import { nativePackageSchema, NATIVE_PROTOCOL_VERSION } from '@demicodes/command-protocol'
import { browserOperations } from '../../packages/browser-protocol/src/index'

const metadataSchema = z.object({
  package: z.object({
    metadata: z.object({ demi: z.object({ id: z.string(), operations: z.array(z.string()) }) }),
  }),
})
const workspaceSchema = z.object({ workspace: z.object({ package: z.object({ version: z.string() }) }) })
const infoSchema = nativePackageSchema.omit({ targets: true })

/**
 * The crates that are native command packages, each released on its own
 * (`native-runtime.md` § Bind an exact package). A crate names its package id
 * and operations in `[package.metadata.demi]`.
 */
export const NATIVE_PACKAGE_CRATES = ['demi-commands', 'demi-claude'] as const
export type NativePackageCrate = (typeof NATIVE_PACKAGE_CRATES)[number]

/** Operations a crate serves that another source of truth declares. */
const operationsDeclaredElsewhere: Partial<Record<NativePackageCrate, () => string[]>> = {
  'demi-commands': () => Object.keys(browserOperations).map(name => `browser.${name}`),
}

/** Release and host-only test catalogs use the command program's Cargo metadata. */
export async function readPackageInfo(crate: NativePackageCrate) {
  const [manifestText, workspaceText] = await Promise.all([
    readFile(new URL(`../../crates/${crate}/Cargo.toml`, import.meta.url), 'utf8'),
    readFile(new URL('../../Cargo.toml', import.meta.url), 'utf8'),
  ])
  const manifest = metadataSchema.parse(Bun.TOML.parse(manifestText))
  const workspace = workspaceSchema.parse(Bun.TOML.parse(workspaceText))
  return infoSchema.parse({
    ...manifest.package.metadata.demi,
    operations: [
      ...manifest.package.metadata.demi.operations,
      ...(operationsDeclaredElsewhere[crate]?.() ?? []),
    ],
    version: workspace.workspace.package.version,
    protocolVersion: NATIVE_PROTOCOL_VERSION,
  })
}

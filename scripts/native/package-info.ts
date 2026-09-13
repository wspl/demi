import { readFile } from 'node:fs/promises'
import { z } from 'zod'
import { nativePackageSchema, NATIVE_PROTOCOL_VERSION } from '@demicodes/command-protocol'

const metadataSchema = z.object({
  package: z.object({
    metadata: z.object({ demi: z.object({ id: z.string(), operations: z.array(z.string()) }) }),
  }),
})
const workspaceSchema = z.object({ workspace: z.object({ package: z.object({ version: z.string() }) }) })
const infoSchema = nativePackageSchema.omit({ targets: true })

/** Release and host-only test catalogs use the command program's Cargo metadata. */
export async function readDemiPackageInfo() {
  const [manifestText, workspaceText] = await Promise.all([
    readFile(new URL('../../crates/demi-commands/Cargo.toml', import.meta.url), 'utf8'),
    readFile(new URL('../../Cargo.toml', import.meta.url), 'utf8'),
  ])
  const manifest = metadataSchema.parse(Bun.TOML.parse(manifestText))
  const workspace = workspaceSchema.parse(Bun.TOML.parse(workspaceText))
  return infoSchema.parse({
    ...manifest.package.metadata.demi,
    version: workspace.workspace.package.version,
    protocolVersion: NATIVE_PROTOCOL_VERSION,
  })
}

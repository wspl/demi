import {
  artifactDigestSchema,
  contentDigest,
  nativeBindingSchema,
  nativePackageSchema,
} from '@demicodes/command-protocol'
import { z } from 'zod'

const commandName = z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_-]*$/)
const jsonSchema = z.record(z.string(), z.unknown())
const leafFields = {
  name: commandName,
  summary: z.string(),
  successOutput: z.string().optional(),
  failureOutput: z.string().optional(),
  runningHint: z.string().optional(),
  input: jsonSchema.optional(),
  positionals: z.array(commandName).optional(),
  stdinField: commandName.optional(),
  restField: commandName.optional(),
  output: z.object({ json: jsonSchema.optional() }).strict().optional(),
}

export const manifestLeafSchema = z.discriminatedUnion('kind', [
  z.object({ ...leafFields, kind: z.literal('rpc') }).strict(),
  z.object({
    ...leafFields,
    kind: z.literal('native'),
    binding: nativeBindingSchema.extend({ descriptorHash: artifactDigestSchema }),
  }).strict(),
])
export type ManifestLeaf = z.infer<typeof manifestLeafSchema>

export interface ManifestGroup {
  name: string
  summary: string
  subcommands: ManifestNode[]
}
export type ManifestNode = ManifestGroup | ManifestLeaf

export const manifestNodeSchema: z.ZodType<ManifestNode> = z.lazy(() =>
  z.union([
    z.object({
      name: commandName,
      summary: z.string(),
      subcommands: z.array(manifestNodeSchema).min(1),
    }).strict(),
    manifestLeafSchema,
  ]),
)

export const manifestSchema = z.object({
  hash: artifactDigestSchema,
  roots: z.record(commandName, z.object({ tree: manifestNodeSchema }).strict()),
  packages: z.record(artifactDigestSchema, nativePackageSchema),
}).strict()
export type Manifest = z.infer<typeof manifestSchema>

export function isManifestGroup(node: ManifestNode): node is ManifestGroup {
  return 'subcommands' in node
}

export function parseManifest(data: unknown): Manifest {
  const manifest = manifestSchema.parse(data)
  const ids = new Set<string>()
  for (const descriptor of Object.values(manifest.packages)) {
    if (ids.has(descriptor.id))
      throw new Error(`Manifest contains duplicate package id: ${descriptor.id}`)
    ids.add(descriptor.id)
  }
  const visit = (node: ManifestNode): void => {
    if (isManifestGroup(node)) {
      const names = new Set<string>()
      for (const child of node.subcommands) {
        if (names.has(child.name))
          throw new Error(`Duplicate manifest subcommand: ${child.name}`)
        names.add(child.name)
        visit(child)
      }
    } else if (node.kind === 'native') {
      const descriptor = manifest.packages[node.binding.descriptorHash]
      if (!descriptor || descriptor.id !== node.binding.package
        || !descriptor.operations.includes(node.binding.operation))
        throw new Error(`Unresolved native binding: ${node.name}`)
    }
  }
  for (const [name, { tree }] of Object.entries(manifest.roots)) {
    if (name !== tree.name)
      throw new Error(`Manifest root name disagrees with tree: ${name}`)
    visit(tree)
  }
  return manifest
}

/** Verifies exact identities after structural validation at a transport boundary. */
export async function verifyManifest(data: unknown): Promise<Manifest> {
  const manifest = parseManifest(data)
  for (const [hash, descriptor] of Object.entries(manifest.packages)) {
    if (await contentDigest(descriptor) !== hash)
      throw new Error(`Native package descriptor hash mismatch: ${descriptor.id}`)
  }
  const { hash, ...body } = manifest
  if (await contentDigest(body) !== hash)
    throw new Error('Command manifest hash mismatch')
  return manifest
}

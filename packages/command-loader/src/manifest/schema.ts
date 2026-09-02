import { z } from 'zod'

/**
 * The manifest (`docs/demi-next/commands.md` § The manifest): every root's
 * tree with JSON Schema in place of zod, and one transpiled module per
 * `runtime` leaf under the hash of its text. It crosses process
 * boundaries (runner socket, HTTP, a cache directory), so it is parsed
 * with this schema on the way in.
 */

const jsonSchema = z.record(z.string(), z.unknown())

const manifestLeafSchema = z.object({
  name: z.string(),
  summary: z.string(),
  kind: z.enum(['rpc', 'runtime']),
  successOutput: z.string().optional(),
  failureOutput: z.string().optional(),
  /** JSON Schema of an object whose properties are the leaf's input fields. */
  input: jsonSchema.optional(),
  positionals: z.array(z.string()).optional(),
  stdinField: z.string().optional(),
  restField: z.string().optional(),
  output: z.object({ json: jsonSchema.optional() }).optional(),
  /** The module hash of a `runtime` leaf. */
  module: z.string().optional(),
})

export type ManifestLeaf = z.infer<typeof manifestLeafSchema>

export interface ManifestGroup {
  name: string
  summary: string
  subcommands: ManifestNode[]
}

export type ManifestNode = ManifestGroup | ManifestLeaf

const manifestNodeSchema: z.ZodType<ManifestNode> = z.lazy(() =>
  z.union([
    z.object({ name: z.string(), summary: z.string(), subcommands: z.array(manifestNodeSchema) }),
    manifestLeafSchema,
  ]),
)

export const manifestSchema = z.object({
  hash: z.string(),
  roots: z.record(z.string(), z.object({ tree: manifestNodeSchema })),
  modules: z.record(z.string(), z.string()),
})

export type Manifest = z.infer<typeof manifestSchema>

export function isManifestGroup(node: ManifestNode): node is ManifestGroup {
  return 'subcommands' in node
}

export function parseManifest(data: unknown): Manifest {
  return manifestSchema.parse(data)
}

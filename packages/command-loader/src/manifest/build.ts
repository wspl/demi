import { isCommandGroup, type Command, type CommandLeaf } from '@demicodes/shell'
import { z } from 'zod'
import type { Manifest, ManifestLeaf, ManifestNode } from './schema'

export interface BuildManifestOptions {
  /** Turns a module's TypeScript text into JavaScript; the backend supplies Bun's transpiler. */
  transpile: (source: string) => string | Promise<string>
}

/**
 * Builds the manifest from assembled root trees: zod becomes JSON Schema,
 * each `runtime` leaf's module text is transpiled and stored under the
 * hash of the result, and the manifest hash covers everything.
 */
export async function buildManifest(roots: readonly Command[], options: BuildManifestOptions): Promise<Manifest> {
  const modules: Record<string, string> = {}
  const moduleHashes = new Map<string, string>()
  const hashModule = async (source: string): Promise<string> => {
    let hash = moduleHashes.get(source)
    if (hash) return hash
    const javascript = await options.transpile(source)
    hash = await sha256(javascript)
    moduleHashes.set(source, hash)
    modules[hash] = javascript
    return hash
  }

  const manifestRoots: Manifest['roots'] = {}
  for (const root of roots) {
    if (manifestRoots[root.name]) throw new Error(`buildManifest: duplicate root "${root.name}"`)
    manifestRoots[root.name] = { tree: await manifestNode(root, hashModule) }
  }
  const body = { roots: manifestRoots, modules }
  return { hash: await sha256(JSON.stringify(body)), ...body }
}

async function manifestNode(command: Command, hashModule: (source: string) => Promise<string>): Promise<ManifestNode> {
  if (isCommandGroup(command)) {
    const subcommands: ManifestNode[] = []
    for (const child of command.subcommands) subcommands.push(await manifestNode(child, hashModule))
    return { name: command.name, summary: command.summary, subcommands }
  }
  return manifestLeaf(command, command.kind === 'runtime' ? await hashModule(command.module) : undefined)
}

function manifestLeaf(leaf: CommandLeaf, module: string | undefined): ManifestLeaf {
  const node: ManifestLeaf = { name: leaf.name, summary: leaf.summary, kind: leaf.kind }
  if (leaf.successOutput !== undefined) node.successOutput = leaf.successOutput
  if (leaf.failureOutput !== undefined) node.failureOutput = leaf.failureOutput
  if (leaf.input) node.input = z.toJSONSchema(z.object(leaf.input))
  if (leaf.positionals) node.positionals = [...leaf.positionals]
  if (leaf.stdinField !== undefined) node.stdinField = leaf.stdinField
  if (leaf.restField !== undefined) node.restField = leaf.restField
  if (leaf.output?.json) node.output = { json: z.toJSONSchema(leaf.output.json) }
  if (module !== undefined) node.module = module
  return node
}

export async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('')
}

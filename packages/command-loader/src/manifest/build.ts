import {
  contentDigest,
  nativePackageSchema,
  type NativePackage,
} from '@demicodes/command-protocol'
import {
  isCommandGroup,
  validateCommandTree,
  type Command,
  type CommandLeaf,
} from '@demicodes/shell'
import { z } from 'zod'
import { parseManifest, type Manifest, type ManifestLeaf, type ManifestNode } from './schema'

export interface BuildManifestOptions {
  packages: readonly NativePackage[]
}

/** Serializes declarations and exact native package bindings without implementation code. */
export async function buildManifest(
  roots: readonly Command[],
  options: BuildManifestOptions,
): Promise<Manifest> {
  const packages: Manifest['packages'] = {}
  const byId = new Map<string, string>()
  for (const value of options.packages) {
    const descriptor = nativePackageSchema.parse(value)
    if (byId.has(descriptor.id))
      throw new Error(`Duplicate native package id: ${descriptor.id}`)
    const hash = await contentDigest(descriptor)
    packages[hash] = descriptor
    byId.set(descriptor.id, hash)
  }
  const manifestRoots: Manifest['roots'] = {}
  for (const root of roots) {
    if (manifestRoots[root.name])
      throw new Error(`buildManifest: duplicate root "${root.name}"`)
    validateCommandTree(root, root.name)
    manifestRoots[root.name] = { tree: manifestNode(root, byId) }
  }
  const body = { roots: manifestRoots, packages }
  return parseManifest({ hash: await contentDigest(body), ...body })
}

function manifestNode(command: Command, packages: ReadonlyMap<string, string>): ManifestNode {
  if (isCommandGroup(command)) {
    return {
      name: command.name,
      summary: command.summary,
      subcommands: command.subcommands.map(child => manifestNode(child, packages)),
    }
  }
  return manifestLeaf(command, packages)
}

function manifestLeaf(leaf: CommandLeaf, packages: ReadonlyMap<string, string>): ManifestLeaf {
  const common = {
    name: leaf.name,
    summary: leaf.summary,
    ...(leaf.successOutput !== undefined ? { successOutput: leaf.successOutput } : {}),
    ...(leaf.failureOutput !== undefined ? { failureOutput: leaf.failureOutput } : {}),
    ...(leaf.runningHint !== undefined ? { runningHint: leaf.runningHint } : {}),
    ...(leaf.input ? { input: z.toJSONSchema(z.object(leaf.input), { io: 'input' }) } : {}),
    ...(leaf.positionals ? { positionals: [...leaf.positionals] } : {}),
    ...(leaf.stdinField !== undefined ? { stdinField: leaf.stdinField } : {}),
    ...(leaf.restField !== undefined ? { restField: leaf.restField } : {}),
    ...(leaf.output?.json ? { output: { json: z.toJSONSchema(leaf.output.json) } } : {}),
  }
  if (leaf.kind === 'rpc')
    return { ...common, kind: 'rpc' }
  const descriptorHash = packages.get(leaf.binding.package)
  if (!descriptorHash)
    throw new Error(`Native package is not configured: ${leaf.binding.package}`)
  return { ...common, kind: 'native', binding: { ...leaf.binding, descriptorHash } }
}

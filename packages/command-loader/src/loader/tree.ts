import { withoutUndefined } from '@demicodes/utils'
import { runtimeModule, type Command, type CommandInputSpec, type CommandLeaf } from '@demicodes/shell'
import { z } from 'zod'
import { isManifestGroup, type Manifest, type ManifestLeaf, type ManifestNode } from '../manifest/schema'
import type { RpcTransport } from './rpc'

/**
 * The manifest as command trees again: JSON Schema back to zod (path marks
 * and descriptions included), a `runtime` leaf carrying its transpiled
 * module, an `rpc` leaf whose handler forwards to the transport. The
 * shell's parser, help renderer and runner then serve every embedder from
 * this one tree shape.
 */
export function treeFromManifest(manifest: Manifest, rpc: RpcTransport | undefined): Command[] {
  return Object.entries(manifest.roots).map(([root, { tree }]) => commandFromNode(root, tree, manifest.modules, rpc))
}

function commandFromNode(root: string, node: ManifestNode, modules: Record<string, string>, rpc: RpcTransport | undefined): Command {
  if (isManifestGroup(node)) {
    return { name: node.name, summary: node.summary, subcommands: node.subcommands.map((child) => commandFromNode(root, child, modules, rpc)) }
  }
  return leafFromNode(root, node, modules, rpc)
}

function leafFromNode(root: string, node: ManifestLeaf, modules: Record<string, string>, rpc: RpcTransport | undefined): CommandLeaf {
  const base = {
    name: node.name,
    summary: node.summary,
    ...(node.successOutput !== undefined ? { successOutput: node.successOutput } : {}),
    ...(node.failureOutput !== undefined ? { failureOutput: node.failureOutput } : {}),
    ...(node.input ? { input: inputFromJsonSchema(node.input) } : {}),
    ...(node.positionals ? { positionals: [...node.positionals] } : {}),
    ...(node.stdinField !== undefined ? { stdinField: node.stdinField } : {}),
    ...(node.restField !== undefined ? { restField: node.restField } : {}),
    ...(node.output?.json ? { output: { json: z.fromJSONSchema(node.output.json as Parameters<typeof z.fromJSONSchema>[0]) } } : {}),
  }
  if (node.kind === 'runtime') {
    const javascript = node.module === undefined ? undefined : modules[node.module]
    if (javascript === undefined) throw new Error(`manifest: runtime leaf "${node.name}" has no module`)
    return { ...base, kind: 'runtime', module: runtimeModule(javascript) }
  }
  return {
    ...base,
    kind: 'rpc',
    run: async (ctx) => {
      if (!rpc) throw new Error(`"${ctx.parsed.path.join(' ')}" is an rpc command and this embedder has no rpc transport`)
      return rpc({
        root,
        path: ctx.parsed.path,
        argv: ctx.argv,
        // Absent optionals leave no key: the wire carries `undefined` as nil.
        args: withoutUndefined(ctx.parsed.values),
        json: ctx.parsed.json,
        stdin: ctx.stdin.bytes,
        cwd: ctx.cwd,
        env: ctx.env,
        io: ctx.io,
        signal: ctx.signal,
        stdinStream: ctx.stdinStream,
      })
    },
  }
}

function inputFromJsonSchema(schema: Record<string, unknown>): CommandInputSpec {
  const object = z.fromJSONSchema(schema as Parameters<typeof z.fromJSONSchema>[0])
  if (!(object instanceof z.ZodObject)) throw new Error('manifest: a leaf input schema must describe an object')
  return { ...object.shape }
}

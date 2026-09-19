import { withoutUndefined } from '@demicodes/utils'
import {
  validateCommandTree,
  type Command,
  type CommandInputSpec,
  type CommandLeaf
} from '@demicodes/shell'
import { z } from 'zod'
import {
  isManifestGroup,
  type Manifest,
  type ManifestLeaf,
  type ManifestNode
} from '../manifest/schema'
import type { RpcTransport } from './rpc'

/** Reconstructs declarations and application RPC forwarding from a manifest. */
export function treeFromManifest(
  manifest: Manifest,
  rpc: RpcTransport | undefined
): Command[] {
  return Object.entries(manifest.roots).map(([root, { tree }]) => {
    const command = commandFromNode(
      root,
      tree,
      rpc
    )
    validateCommandTree(command, root)
    return command
  })
}

function commandFromNode(
  root: string,
  node: ManifestNode,
  rpc: RpcTransport | undefined
): Command {
  if (isManifestGroup(node)) {
    return {
      name: node.name,
      summary: node.summary,
      subcommands: node.subcommands.map((child) => commandFromNode(
        root,
        child,
        rpc
      ))
    }
  }
  return leafFromNode(root, node, rpc)
}

function leafFromNode(
  root: string,
  node: ManifestLeaf,
  rpc: RpcTransport | undefined
): CommandLeaf {
  const base = {
    name: node.name,
    summary: node.summary,
    ...(node.successOutput !== undefined
      ? { successOutput: node.successOutput }
      : {}),
    ...(node.failureOutput !== undefined
      ? { failureOutput: node.failureOutput }
      : {}),
    ...(node.runningHint
      !== undefined ? { runningHint: node.runningHint } : {}),
    ...(node.input ? { input: inputFromJsonSchema(node.input) } : {}),
    ...(node.positionals ? { positionals: [...node.positionals] } : {}),
    ...(node.stdinField !== undefined ? { stdinField: node.stdinField } : {}),
    ...(node.restField !== undefined ? { restField: node.restField } : {}),
    ...(node.output?.json ? {
      output: {
        json: z.fromJSONSchema(node.output.json as Parameters<typeof z.fromJSONSchema>[0])
      }
    } : {}),
  }
  if (node.kind === 'native') {
    return {
      ...base,
      kind: 'native',
      binding: { package: node.binding.package, operation: node.binding.operation },
    }
  }
  return {
    ...base,
    kind: 'rpc',
    run: async (ctx) => {
      if (!rpc)
        throw new Error(
          `"${ctx.parsed.path.join(' ')}" is an rpc command and this embedder has no rpc transport`
        )
      return rpc({
        root,
        path: ctx.parsed.path,
        argv: ctx.argv,
        // Absent optionals leave no key: the wire carries `undefined` as nil.
        args: withoutUndefined(ctx.parsed.values),
        json: ctx.parsed.json,
        stdin: ctx.stdin,
        cwd: ctx.cwd,
        env: ctx.env,
        context: ctx.context,
        io: ctx.io,
        signal: ctx.signal,
        stdinStream: ctx.stdinStream,
      })
    },
  }
}

function inputFromJsonSchema(schema: Record<string, unknown>): CommandInputSpec {
  const object = z.fromJSONSchema(schema as Parameters<typeof z.fromJSONSchema>[0])
  if (!(object instanceof z.ZodObject))
    throw new Error('manifest: a leaf input schema must describe an object')
  return { ...object.shape }
}

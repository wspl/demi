import {
  isCommandGroup,
  resolveCommand,
  type Command,
  type CommandIO,
  type CommandResult,
  type CommandStorage,
  type Host,
} from '@demicodes/shell'

/**
 * One `rpc` invocation as the loader hands it to a transport: the parsed
 * arguments travel; the pipe, stdout, stderr and the post-start stdin
 * stream are attached for the transport to carry (`runner.md` § Pipes).
 */
export interface RpcInvocation {
  root: string
  /** Path from the root through the leaf, root name included. */
  path: string[]
  argv: string[]
  args: Record<string, unknown>
  json: boolean
  /** The pipe, finite; `null` when the process has none on fd 0 or a `stdinField` consumed it. */
  stdin: AsyncIterable<Uint8Array> | null
  cwd: string
  env: Record<string, string>
  io: CommandIO
  signal: AbortSignal
  stdinStream: AsyncIterable<Uint8Array>
}

export type RpcTransport = (invocation: RpcInvocation) => Promise<CommandResult>

/**
 * The backend's transport: the trees that declared the `rpc` handlers are in
 * this process, so an invocation runs its handler directly.
 */
export function inProcessRpc(roots: readonly Command[], deps: { storage: CommandStorage; host: Host }): RpcTransport {
  return async (invocation) => {
    const root = roots.find((candidate) => candidate.name === invocation.root)
    if (!root) throw new Error(`rpc: unknown root "${invocation.root}"`)
    const leaf = resolveCommand(root, invocation.path)
    if (isCommandGroup(leaf) || leaf.kind !== 'rpc') throw new Error(`rpc: "${invocation.path.join(' ')}" is not an rpc leaf`)
    return leaf.run({
      argv: invocation.argv,
      parsed: { path: invocation.path, help: false, values: invocation.args, json: invocation.json },
      stdin: invocation.stdin,
      env: invocation.env,
      cwd: invocation.cwd,
      io: invocation.io,
      storage: deps.storage,
      host: deps.host,
      signal: invocation.signal,
      stdinStream: invocation.stdinStream,
    })
  }
}

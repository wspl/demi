import { buildManifest, createLoader, inMemorySource, inProcessRpc, rootPaths } from '@demicodes/command-loader'
import { AgentSessionCommandStorage, type CommandRegistry, type Host, type ShellEnvironment, type ShellEnvironmentOptions } from '@demicodes/shell'
import { HostlessEnvironment } from '@demicodes/host-virtual'
import { HOSTLESS_HOME, HOSTLESS_NAMESPACE } from './scoped-transport'

/** Every hostless file belongs to the session user (`sessions-and-targets.md` § The namespace). */
export const HOSTLESS_IDENTITY = { user: 'demi', group: 'demi' } as const

const transpiler = new Bun.Transpiler({ loader: 'ts', target: 'browser' })
const transpiled = new Map<string, string>()

/** The backend's transpiler for the manifest build: Bun, memoized per module text. */
export function transpileCommandModule(source: string): string {
  let javascript = transpiled.get(source)
  if (javascript === undefined) {
    javascript = transpiler.transformSync(source)
    transpiled.set(source, javascript)
  }
  return javascript
}

/**
 * The hostless conversation's shell: the session's command trees become a
 * manifest, the loader runs `runtime` modules against the conversation's
 * Host and `rpc` handlers in this process, and tinybash runs the scripts.
 */
export async function createHostlessShell(ctx: {
  agentSessionId: string
  host: Host
  commands: CommandRegistry
  shell: ShellEnvironmentOptions
}): Promise<ShellEnvironment> {
  const roots = ctx.commands.list()
  const manifest = await buildManifest(roots, { transpile: transpileCommandModule })
  const loader = await createLoader({
    source: inMemorySource(manifest),
    host: ctx.host,
    rpc: inProcessRpc(roots, { storage: new AgentSessionCommandStorage(ctx.host.store, ctx.agentSessionId), host: ctx.host }),
  })
  return new HostlessEnvironment({
    ...ctx.shell,
    host: ctx.host,
    roots: rootPaths(loader.roots),
    dispatch: (root, argv, io) => loader.dispatch(root, argv, io),
    home: HOSTLESS_HOME,
    namespace: HOSTLESS_NAMESPACE,
    identity: HOSTLESS_IDENTITY,
  })
}

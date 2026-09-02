// Test helpers for packages that need a shell or a Host without a machine:
// the hostless shell environment composed over any Host — the manifest
// built from a command registry with Bun's transpiler, the loader with
// `rpc` in process, tinybash running the scripts — the `probe` root that
// stands in for `sleep` and `read`, and `LocalHost`, the Host over this
// Node process's machine. Shipped as `@demicodes/host-virtual/testing`,
// never imported by runtime code.
import { buildManifest, createLoader, inMemorySource, inProcessRpc, rootPaths } from '@demicodes/command-loader'
import { AgentSessionCommandStorage, type Command, type CommandRegistry, type Host, type ShellEnvironment, type ShellEnvironmentOptions } from '@demicodes/shell'
import { delay, utf8Lines } from '@demicodes/utils'
import { z } from 'zod'
import { HostlessEnvironment } from './hostless-environment'

export { LocalHost, type LocalHostOptions } from './testing/local-host'

const transpiler = new Bun.Transpiler({ loader: 'ts', target: 'browser' })

export interface HostlessShellOptions extends ShellEnvironmentOptions {
  host: Host
  commands: CommandRegistry
  /** The session `rpc` handlers run under (default `test-session`). */
  agentSessionId?: string
  /** `$HOME` and the first namespace prefix; default the Host's `defaultCwd`. */
  home?: string
  /** Absolute prefixes a script may touch; default `[home]`. */
  namespace?: readonly string[]
}

/** A hostless shell over `host`: what the backend composes for a hostless conversation, over any Host a test has. */
export async function hostlessShell(options: HostlessShellOptions): Promise<ShellEnvironment> {
  const { host, commands, agentSessionId = 'test-session', home = host.defaultCwd, namespace = [home], ...shell } = options
  const roots = commands.list()
  const manifest = await buildManifest(roots, { transpile: (source) => transpiler.transformSync(source) })
  const loader = await createLoader({
    source: inMemorySource(manifest),
    host,
    rpc: inProcessRpc(roots, { storage: new AgentSessionCommandStorage(host.store, agentSessionId), host }),
  })
  return new HostlessEnvironment({
    ...shell,
    host,
    roots: rootPaths(loader.roots),
    dispatch: (root, argv, io) => loader.dispatch(root, argv, io),
    home,
    namespace,
    identity: { user: 'demi', group: 'demi' },
  })
}

/** The `AgentServer.shellEnvironment` factory for tests: every Host gets a hostless shell. */
export function hostlessShellFactory(ctx: { agentSessionId: string; host: Host; commands: CommandRegistry; shell: ShellEnvironmentOptions }): Promise<ShellEnvironment> {
  return hostlessShell({ ...ctx.shell, host: ctx.host, commands: ctx.commands, agentSessionId: ctx.agentSessionId })
}

/**
 * A root command for tests that need a script to wait or to read its live
 * stdin — what `sleep` and `read` did when a real shell ran the tests. The
 * hostless shell gives the script's stdin to root commands only, and a
 * builtin never sleeps.
 */
export function probeCommand(): Command {
  return {
    name: 'probe',
    summary: 'Test probes: hold for a while, or echo the first line of the live stdin.',
    subcommands: [
      {
        name: 'hold',
        summary: 'Wait `ms` milliseconds (aborted with the command).',
        input: { ms: z.coerce.number() },
        positionals: ['ms'],
        kind: 'rpc',
        run: async ({ parsed, signal }) => {
          const ms = parsed.values.ms as number
          const held = delay(ms)
          const aborted = new Promise<'aborted'>((resolve) => signal.addEventListener('abort', () => resolve('aborted'), { once: true }))
          return (await Promise.race([held.then(() => 'held' as const), aborted])) === 'aborted' ? { exitCode: 130 } : { exitCode: 0 }
        },
      },
      {
        name: 'stdin',
        summary: 'Print the first line written to the running command (`--delay` waits before printing).',
        input: { delay: z.coerce.number().optional() },
        kind: 'rpc',
        run: async ({ parsed, stdinStream, io }) => {
          for await (const line of utf8Lines(stdinStream)) {
            const wait = parsed.values.delay as number | undefined
            if (wait) await delay(wait)
            await io.stdout(line)
            return { exitCode: 0 }
          }
          return { exitCode: 1 }
        },
      },
    ],
  }
}

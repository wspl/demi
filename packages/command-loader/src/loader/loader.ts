import { runRegisteredCommand, type Command, type DispatchIO, type Host } from '@demicodes/shell'
import { errorMessage } from '@demicodes/utils'
import type { Manifest } from '../manifest/schema'
import type { RpcTransport } from './rpc'
import { treeFromManifest } from './tree'

/** Where a loader gets its manifest: in memory, a directory, a socket, a URL. */
export interface ManifestSource {
  manifest(): Promise<Manifest>
}

export interface LoaderOptions {
  source: ManifestSource
  /** The Host `runtime` modules run against. */
  host: Host
  /** Carries `rpc` invocations; absent, the embedder serves only `runtime` commands. */
  rpc?: RpcTransport
}

export interface Loader {
  manifest: Manifest
  /** The manifest's roots as command trees; help and `rootPaths` come from here. */
  roots: Command[]
  /** Runs `root argv…` with the given stdio; usage errors print to stderr and exit 1. */
  dispatch(root: string, argv: readonly string[], io: DispatchIO): Promise<number>
}

/**
 * The one place that knows how to run a command (`docs/demi-next/commands.md`
 * § The loader). A `runtime` module runs in this process against `host`;
 * an `rpc` invocation goes to the transport.
 */
export async function createLoader(options: LoaderOptions): Promise<Loader> {
  const manifest = await options.source.manifest()
  const roots = treeFromManifest(manifest, options.rpc)
  return {
    manifest,
    roots,
    dispatch: async (root, argv, io) => {
      const tree = roots.find((candidate) => candidate.name === root)
      if (!tree) {
        await io.stderr(`${root}: not a root command of this manifest\n`)
        return 127
      }
      try {
        const result = await runRegisteredCommand(tree, {
          argv: [root, ...argv],
          stdin: io.stdin,
          env: io.env,
          cwd: io.cwd,
          io: { stdout: io.stdout, stderr: io.stderr },
          host: options.host,
          signal: io.signal,
        })
        return result.exitCode
      } catch (error) {
        await io.stderr(`${root}: ${errorMessage(error)}\n`)
        return 1
      }
    },
  }
}

export function inMemorySource(manifest: Manifest): ManifestSource {
  return { manifest: async () => manifest }
}

import {
  runRegisteredCommand,
  type Command,
  type DispatchIO,
  type Host,
  type NativeExecutor
} from '@demicodes/shell'
import { errorMessage } from '@demicodes/utils'
import type { Manifest } from '../manifest/schema'
import type { RpcTransport } from './rpc'
import type { ManifestSource } from './source'
import { treeFromManifest } from './tree'

export interface LoaderOptions {
  source: ManifestSource
  host: Host
  rpc?: RpcTransport
  native?: NativeExecutor
}

export interface Loader {
  manifest: Manifest
  /** The manifest's roots as command trees; help is derived from them. */
  roots: Command[]
  /**
   * Runs `root argv…` with the given stdio; usage errors print to stderr and
   * exit 1.
   */
  dispatch(
    root: string,
    argv: readonly string[],
    io: DispatchIO
  ): Promise<number>
}

/** Dispatches declarations through the embedder's explicit execution adapters. */
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
          stdinStream: io.stdinStream,
          env: io.env,
          cwd: io.cwd,
          io: { stdout: io.stdout, stderr: io.stderr },
          host: options.host,
          signal: io.signal,
          native: options.native,
          onRunningHint: io.onRunningHint,
        })
        return result.exitCode
      } catch (error) {
        await io.stderr(`${root}: ${errorMessage(error)}\n`)
        return 1
      }
    },
  }
}

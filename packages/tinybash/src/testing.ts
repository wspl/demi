import { concatBytes, emptyByteStream, errorCode } from '@demicodes/utils'
import type { DispatchIO, RootPaths } from './index'

export interface RecordedCall {
  root: string
  argv: string[]
  stdin: string
  cwd: string
}

/**
 * Stub roots for embedders' tests: each call is recorded with its argv and
 * stdin, echoes `<root> <argv...>` on stdout followed by the stdin between
 * `stdin:` and `:end` lines when there was any, and exits 0. `paths` marks which
 * argument positions are paths for the namespace check.
 */
export function stubRoots(spec: Record<string, { paths?: (argv: readonly string[]) => readonly string[] }>): {
  roots: Map<string, RootPaths>
  dispatch: (root: string, argv: string[], io: DispatchIO) => Promise<number>
  calls: RecordedCall[]
} {
  const calls: RecordedCall[] = []
  const roots = new Map<string, RootPaths>()
  for (const [name, entry] of Object.entries(spec)) roots.set(name, entry.paths ?? (() => []))
  return {
    roots,
    calls,
    dispatch: async (root, argv, io) => {
      const chunks: Uint8Array[] = []
      try {
        for await (const chunk of io.stdin ?? emptyByteStream()) chunks.push(chunk)
      } catch (error) {
        // The bash-side stub reads stdin with `cat`, which reports a directory this way and goes on.
        if (errorCode(error) !== 'EISDIR') throw error
        await io.stderr('cat: -: Is a directory\n')
      }
      const stdin = new TextDecoder().decode(concatBytes(chunks))
      calls.push({ root, argv, stdin, cwd: io.cwd })
      await io.stdout(`${[root, ...argv].join(' ')}\n`)
      if (stdin.length > 0) await io.stdout(`stdin:\n${stdin}:end\n`)
      return 0
    },
  }
}

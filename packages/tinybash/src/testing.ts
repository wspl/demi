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
      for await (const chunk of io.stdin) chunks.push(chunk)
      const stdin = new TextDecoder().decode(concat(chunks))
      calls.push({ root, argv, stdin, cwd: io.cwd })
      await io.stdout(`${[root, ...argv].join(' ')}\n`)
      if (stdin.length > 0) await io.stdout(`stdin:\n${stdin}:end\n`)
      return 0
    },
  }
}

function concat(chunks: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0))
  let offset = 0
  for (const chunk of chunks) {
    out.set(chunk, offset)
    offset += chunk.length
  }
  return out
}

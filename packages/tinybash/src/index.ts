import type { CommandIO, HostFileSystem } from '@demicodes/shell'
import type { Script } from './grammar/ast'
import { parseScript } from './grammar/parser'
import { type RootPaths, checkScript } from './outside/check'
import { OutsideError, type OutsideReason, refusalMessage } from './outside/reasons'
import type { DispatchIO } from '@demicodes/shell'
import { type ShellState, executeScript } from './exec/executor'

export type { OutsideReason, RootPaths, DispatchIO, ShellState }
export type { Script } from './grammar/ast'
export { refusalMessage } from './outside/reasons'

export interface TinybashInput {
  script: string
  /** Root names → the path-typed arguments of an invocation (argv without the root name). */
  roots: ReadonlyMap<string, RootPaths>
  /** Absolute prefixes the script may touch, e.g. `['/home/demi', '/tmp']`; `/dev/null` is always allowed. */
  namespace: readonly string[]
  /** Runs a root command; `argv` excludes the root name. */
  dispatch: (root: string, argv: string[], io: DispatchIO) => Promise<number>
  fs: HostFileSystem
  /** The session shell state; `cd` and assignments mutate it. */
  state: ShellState
  io: CommandIO
  /** Owner names `ls -l` shows for every file. */
  identity: { user: string; group: string }
  signal?: AbortSignal
}

export type TinybashOutside = { kind: 'outside'; reason: OutsideReason; message: string }
export type TinybashResult = { kind: 'ran'; exitCode: number } | TinybashOutside

export type ParseResult = { kind: 'script'; script: Script } | TinybashOutside

/**
 * Parses and checks a script without running anything: the whole parse-first
 * decision, so an embedder can hand the script elsewhere before touching the
 * loader or the Host. Path conditions that need the filesystem (a glob whose
 * expansion leaves the namespace) cannot arise: a glob's static prefix is
 * checked here and its matches lie under that prefix.
 */
export function parseTinybash(script: string, roots: ReadonlyMap<string, RootPaths>, namespace: readonly string[], state: Readonly<ShellState>): ParseResult {
  try {
    const parsed = parseScript(script)
    checkScript(parsed, { roots, namespace, scope: { home: state.home, cwd: state.cwd, vars: state.vars } })
    return { kind: 'script', script: parsed }
  } catch (error) {
    if (error instanceof OutsideError) return { kind: 'outside', reason: error.reason, message: refusalMessage(error.reason) }
    throw error
  }
}

export async function runTinybash(input: TinybashInput): Promise<TinybashResult> {
  const parsed = parseTinybash(input.script, input.roots, input.namespace, input.state)
  if (parsed.kind === 'outside') return parsed
  const exitCode = await executeScript(parsed.script, {
    fs: input.fs,
    state: input.state,
    roots: input.roots,
    dispatch: input.dispatch,
    identity: input.identity,
    stdout: (data) => input.io.stdout(data),
    stderr: (data) => input.io.stderr(data),
    signal: input.signal,
  })
  return { kind: 'ran', exitCode }
}

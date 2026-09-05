import { shareByteStream } from '@demicodes/utils'
import type { DispatchIO, RootPaths, TinybashFs, TinybashIO } from './host'
import type { Script } from './grammar/ast'
import { parseScript } from './grammar/parser'
import { checkScript } from './outside/check'
import { OutsideError, type OutsideReason, refusalMessage } from './outside/reasons'
import { type ShellState, executeScript } from './exec/executor'

export type { OutsideReason, ShellState }
export type { DispatchIO, RootPaths, TinybashDirent, TinybashFs, TinybashIO, TinybashStat, TinybashWriter } from './host'
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
  fs: TinybashFs
  /** The session shell state; `cd` and assignments mutate it. */
  state: ShellState
  io: TinybashIO
  /** The script's stdin: what the caller writes while it runs. Every command whose stdin is not redirected reads it, as under bash: builtins directly, root commands as their live stream. */
  stdin?: AsyncIterable<Uint8Array>
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
 * loader. Every path is checked under every shell state the script can
 * reach (`outside/check.ts`); `fs`, when given, lets the check decide a `cd`
 * that nothing before it could have affected, so fewer scripts are handed to
 * a machine for a `..` after a `cd` that plainly succeeds.
 */
export async function parseTinybash(script: string, roots: ReadonlyMap<string, RootPaths>, namespace: readonly string[], state: Readonly<ShellState>, fs?: TinybashFs): Promise<ParseResult> {
  try {
    const parsed = parseScript(script)
    await checkScript(parsed, { roots, namespace, scope: { home: state.home, cwd: state.cwd, vars: state.vars }, fs })
    return { kind: 'script', script: parsed }
  } catch (error) {
    if (error instanceof OutsideError) return { kind: 'outside', reason: error.reason, message: refusalMessage(error.reason) }
    throw error
  }
}

export async function runTinybash(input: TinybashInput): Promise<TinybashResult> {
  const parsed = await parseTinybash(input.script, input.roots, input.namespace, input.state, input.fs)
  if (parsed.kind === 'outside') return parsed
  const exitCode = await executeScript(parsed.script, {
    fs: input.fs,
    state: input.state,
    roots: input.roots,
    dispatch: input.dispatch,
    identity: input.identity,
    stdout: (data) => input.io.stdout(data),
    stderr: (data) => input.io.stderr(data),
    stdin: input.stdin ? shareByteStream(input.stdin) : undefined,
    signal: input.signal,
  })
  return { kind: 'ran', exitCode }
}

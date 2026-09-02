import type { TinybashFs } from '../host'
import type { Writer } from '../exec/stream'
import { throwIfAborted } from '@demicodes/utils'

/** What a builtin sees: the same world a root command's `ctx` is built from. */
export interface BuiltinContext {
  argv: readonly string[]
  stdin: AsyncIterable<Uint8Array>
  stdout: Writer
  stderr: Writer
  fs: TinybashFs
  cwd: string
  home: string
  env: Readonly<Record<string, string>>
  /** Owner names for `ls -l`; every hostless file belongs to the session user. */
  identity: { user: string; group: string }
  signal?: AbortSignal
  /** The script line the command is on, for bash's own error prefix. */
  line: number
  /** `cd` and assignments-only commands mutate the shell through this. */
  shell: { setCwd(path: string): void }
}

export type Builtin = (ctx: BuiltinContext) => Promise<number>

/**
 * A builtin that reads a stream or walks a tree calls this on every step, so
 * a cancelled session stops it mid-way; the executor turns the throw into
 * exit status 130, as a keyboard interrupt would.
 */
export function checkCancelled(ctx: Pick<BuiltinContext, 'signal'>): void {
  if (ctx.signal) throwIfAborted(ctx.signal)
}

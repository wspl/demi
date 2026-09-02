import type { HostFileSystem } from '@demicodes/shell'
import type { Writer } from '../exec/stream'

/** What a builtin sees: the same world a root command's `ctx` is built from. */
export interface BuiltinContext {
  argv: readonly string[]
  stdin: AsyncIterable<Uint8Array>
  stdout: Writer
  stderr: Writer
  fs: HostFileSystem
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

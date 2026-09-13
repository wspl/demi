import type { NativeBinding } from '@demicodes/command-protocol'

/** A writer for stdout or stderr: text is UTF-8, bytes pass through. */
export type CommandWriter = (data: string | Uint8Array) => Promise<void> | void

export interface CommandResult {
  exitCode: number
}

/** A validated native invocation; its execution adapter owns the package catalog. */
export interface NativeInvocation {
  binding: NativeBinding
  args: Record<string, unknown>
  cwd: string
  env: Record<string, string>
  stdin: AsyncIterable<Uint8Array>
  stdout: CommandWriter
  stderr: CommandWriter
  signal: AbortSignal
}

export type NativeExecutor = (invocation: NativeInvocation) => Promise<CommandResult>

export interface DispatchIO {
  /**
   * The pipe: a pipeline, heredoc, `<` file. Finite. Absent when fd 0 is not a
   * pipe.
   */
  stdin?: AsyncIterable<Uint8Array>
  /**
   * The script's own stdin, when this command's stdin is not redirected:
   * what the shell's caller writes after the command started (`shell_write`).
   * Live — it ends when the caller ends it, never on its own.
   */
  stdinStream?: AsyncIterable<Uint8Array>
  stdout: CommandWriter
  stderr: CommandWriter
  cwd: string
  env: Record<string, string>
  signal?: AbortSignal
  /**
   * The executing leaf's hint, cleared when it settles; help and invalid
   * invocations never set one.
   */
  onRunningHint?: (hint: string | undefined) => void | Promise<void>
}

/**
 * Public spawn shape for running the Claude Code CLI on an injected execution
 * target: a structural subset of `Host.process.spawn` from `@demicodes/shell`
 * (which this package must not import) — a real `host.process.spawn` is
 * directly assignable to `ClaudeSpawn`.
 */

/** The executable and directories on the machine running the CLI. */
export interface ClaudeProcessPlace {
  command: string
  cwd: string
  /** The CLI's configuration home (`CLAUDE_CONFIG_DIR`). */
  configDir: string
}

export interface ClaudeSpawnParams {
  command: string
  args?: string[]
  cwd?: string
  env?: Record<string, string | undefined>
  /** The process is kept between turns and is not itself work (`Host.process`). */
  retained?: boolean
}

export interface ClaudeSpawnHandle {
  stdout: AsyncIterable<Uint8Array>
  stderr: AsyncIterable<Uint8Array>
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(signal?: string): Promise<void>
  wait(): Promise<ClaudeSpawnExit>
}

export interface ClaudeSpawnExit {
  exitCode: number | null
  signal?: string
  spawnError?: { kind: string }
}

export type ClaudeSpawn = (
  params: ClaudeSpawnParams
) => Promise<ClaudeSpawnHandle>

/**
 * Public spawn shape for running the Claude Code CLI on an injected execution
 * target: a structural subset of `Host.process.spawn` from `@demicodes/shell`
 * (which this package must not import) — a real `host.process.spawn` is
 * directly assignable to `ClaudeSpawn`.
 */

export interface ClaudeSpawnParams {
  command: string
  args?: string[]
  cwd?: string
  env?: Record<string, string | undefined>
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

export type ClaudeSpawn = (params: ClaudeSpawnParams) => Promise<ClaudeSpawnHandle>

import type { Interpreter, InterpreterState } from '@demicodes/just-bash/interpreter'
import type { CommandRegistry as ForkCommandRegistry, ExecResult as ForkExecResult } from '@demicodes/just-bash/types'
import type { HostSpawnHandle } from './host'
import type { HostBackedFileSystem } from './host-fs'
import type { BashAuditEvent, CommandMetadataRecord, ShellOutputRecordChunk } from './environment'

export interface ExecAccumulator {
  stdout: string
  stderr: string
  audit: BashAuditEvent[]
  commandMetadata: CommandMetadataRecord[]
}

export interface ShellSession {
  id: string
  agentSessionId: string | null
  commandStorageId: string
  state: InterpreterState
  fs: HostBackedFileSystem
  interpreter: Interpreter
  forkCommands: ForkCommandRegistry
  accumulator: ExecAccumulator
  foreground?: ForegroundProcess
  activeCommandId?: string
  backgroundJobs: Map<number, BackgroundJob>
  nextBackgroundJobId: number
  pendingExec?: Promise<ForkExecResult | Error>
  foregroundWaiters: Set<(foreground: ForegroundProcess) => void>
  exited: boolean
  abortController?: AbortController
}

export interface BackgroundJob {
  id: number
  command: string
  args: string[]
  display: string
  cwd: string
  handle: HostSpawnHandle
  stdoutBuffer: string
  stderrBuffer: string
  stdoutPump: Promise<void>
  stderrPump: Promise<void>
  exitPromise: Promise<{ exitCode: number | null; signal?: string }>
}

export interface ForegroundProcess {
  commandId: string
  command: string
  args: string[]
  cwd: string
  handle: HostSpawnHandle
  startedAt: number
  lastOutputAt: number
  /** Everything the process wrote, including redirected output — this is what
   * the interpreter observes as the command's stdout/stderr. */
  rawStdoutBuffer: string
  /** Raw stdout byte chunks for byte-clean pipeline continuation. */
  rawStdoutBytes: Uint8Array[]
  rawStderrBuffer: string
  /** Output routed to the visible sinks only (redirections excluded) — this is
   * what command records and model previews show. */
  stdoutBuffer: string
  stderrBuffer: string
  /** Interleaved visible chunks with running byte offsets, for merged replay. */
  outputChunks: ShellOutputRecordChunk[]
  outputBytes: number
  audit: BashAuditEvent[]
  stdoutPump: Promise<void>
  stderrPump: Promise<void>
  exitPromise: Promise<{ exitCode: number | null; signal?: string }>
  outputSinks: Record<1 | 2, ForegroundSink>
  abortController: AbortController
}

export interface ForegroundSink {
  kind: 'visible' | 'file' | 'null'
  fd?: 1 | 2
  path?: string
  append?: boolean
  bytes: Uint8Array[]
}

export type BoundaryOutcome =
  | { kind: 'foreground_appeared'; foreground: ForegroundProcess }
  | { kind: 'timeout' }
  | { kind: 'aborted' }

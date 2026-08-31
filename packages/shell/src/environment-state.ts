import type { Interpreter, InterpreterState } from '@demicodes/just-bash/interpreter'
import type { CommandRegistry as ForkCommandRegistry, ExecResult as ForkExecResult } from '@demicodes/just-bash/types'
import type { HostCwd, HostSpawnExit, HostSpawnHandle } from './host'
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
  cwdHandle: HostCwd
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
  /** Chars discarded from the head of each buffer once it outgrew the capture limit. */
  droppedStdoutChars: number
  droppedStderrChars: number
  stdoutPump: Promise<void>
  stderrPump: Promise<void>
  exitPromise: Promise<HostSpawnExit>
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
  /** Total bytes ingested across both streams; the capture limit judges this. */
  capturedBytes: number
  /** Set when the capture limit was breached: the process is killed and further chunks dropped. */
  captureOverflowed: boolean
  audit: BashAuditEvent[]
  stdoutPump: Promise<void>
  stderrPump: Promise<void>
  exitPromise: Promise<HostSpawnExit>
  outputSinks: Record<1 | 2, ForegroundSink>
  abortController: AbortController
  /** The resolved registered command's `runningHint`, surfaced on running command statuses. */
  runningHint?: string
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

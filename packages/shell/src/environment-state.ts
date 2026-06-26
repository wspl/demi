import type { Interpreter, InterpreterState } from 'just-bash/interpreter'
import type { CommandRegistry as ForkCommandRegistry, ExecResult as ForkExecResult } from 'just-bash/types'
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
  commandScopeId: string
  state: InterpreterState
  fs: HostBackedFileSystem
  interpreter: Interpreter
  forkCommands: ForkCommandRegistry
  accumulator: ExecAccumulator
  startStdoutBytes: number
  startStderrBytes: number
  totalStdoutBytes: number
  totalStderrBytes: number
  stdoutTail: string
  stderrTail: string
  truncated: boolean
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
  rawStdoutBuffer: string
  rawStderrBuffer: string
  stdoutBuffer: string
  stderrBuffer: string
  outputChunks: ShellOutputRecordChunk[]
  outputBytes: number
  lastStdoutSnapshot: number
  lastStderrSnapshot: number
  lastRawStdoutBytesSnapshot: number
  lastRawStderrBytesSnapshot: number
  lastStdoutBytesSnapshot: number
  lastStderrBytesSnapshot: number
  rawStdoutBytes: number
  rawStderrBytes: number
  totalStdoutBytes: number
  totalStderrBytes: number
  audit: BashAuditEvent[]
  stdoutPump: Promise<void>
  stderrPump: Promise<void>
  exitPromise: Promise<{ exitCode: number | null; signal?: string }>
  outputSinks: Record<1 | 2, ForegroundSink>
  abortController: AbortController
  outputLimitWaiters: Set<() => void>
  redirectedStdoutBytes: number
  redirectedStderrBytes: number
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
  | { kind: 'output_limit' }
  | { kind: 'timeout' }
  | { kind: 'aborted' }

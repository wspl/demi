/**
 * The shell environment behind the `shell_exec` / `shell_status` /
 * `shell_write` / `shell_abort` tools: one interface, two implementations —
 * `HostlessEnvironment` (`@demicodes/host-virtual`: tinybash over the
 * conversation's store-backed Host) and `RemoteShellEnvironment`
 * (`@demicodes/host-remote`: jobs on a machine's runner). The agent server
 * talks to the interface only.
 */

export interface ShellExecInput {
  script: string
  shellId?: string
  agentSessionId?: string
  timeoutMs?: number
  signal?: AbortSignal
  /**
   * Run in a dedicated one-shot shell instead of the session default shell, so
   * cd/env side effects never leak into other execs sharing the session. The
   * caller owns the shell and should `disposeShell(snapshot.shellId)` when done.
   * Mutually exclusive with `shellId`.
   */
  ephemeral?: boolean
  /**
   * Initial working directory of the shell this exec creates. Requires
   * `ephemeral` — a persistent shell owns its cwd (that is what `cd` is for).
   */
  cwd?: string
}

export interface ShellStatusInput {
  commandId: string
}

export interface ShellWriteInput {
  commandId: string
  stdin: string | Uint8Array
  signal?: AbortSignal
}

export interface ShellAbortInput {
  commandId: string
}

export interface ShellStreamView {
  /** Where the full stream lives on the target; absent when nothing beyond the view is kept (hostless). */
  path?: string
  offset: number
  delta: string
  tail: string
  bytes: number
  truncated: boolean
}

export interface ShellOutputChunk {
  stream: 'stdout' | 'stderr'
  text: string
}

export interface ShellOutputRecordChunk extends ShellOutputChunk {
  offset: number
  bytes: number
}

export interface ShellOutputView {
  /** The target directory holding the output files; absent for hostless. */
  path?: string
  offset: number
  text: string
  tail: string
  chunks: ShellOutputChunk[]
  bytes: number
  truncated: boolean
}

/** Final stdout stream that is not valid UTF-8: raw bytes for the boundary above. */
export interface BinaryStdout {
  data: Uint8Array
  /** True when the stream exceeded the applicable cap; data is capped. */
  truncated: boolean
  /** Total byte count of the un-capped stream. */
  totalBytes: number
  /** The byte ceiling that applied, so the boundary above can name it. */
  limitBytes: number
}

export type ShellCommandStatus =
  | {
      status: 'exited'
      shellId: string
      commandId: string
      /** The directory the target's output files live in; absent when nothing beyond the view is kept (hostless). */
      outputDir?: string
      exitCode: number
      stdout: ShellStreamView
      stderr: ShellStreamView
      output: ShellOutputView
      runningMs: number
      idleMs: number
      /** Present when the final stream was binary (bytes that are not valid UTF-8). */
      binaryStdout?: BinaryStdout
    }
  | {
      status: 'running'
      shellId: string
      commandId: string
      outputDir?: string
      stdout: ShellStreamView
      stderr: ShellStreamView
      output: ShellOutputView
      runningMs: number
      idleMs: number
    }
  | {
      status: 'aborted'
      shellId: string
      commandId: string
      outputDir?: string
      stdout: ShellStreamView
      stderr: ShellStreamView
      output: ShellOutputView
      runningMs: number
      idleMs: number
    }

export interface ShellEnvironment {
  exec(input: ShellExecInput): Promise<ShellCommandStatus>
  status(input: ShellStatusInput): Promise<ShellCommandStatus>
  write(input: ShellWriteInput): Promise<ShellCommandStatus>
  abort(input: ShellAbortInput): Promise<ShellCommandStatus>
  /** Forgets a command and removes its artifact; false when unknown. */
  releaseCommand(commandId: string): Promise<boolean>
  disposeShell(shellId: string): Promise<boolean>
  disposeAllShells(): Promise<void>
  /** The shell with this id, if this environment owns it. */
  getShell(shellId: string): { id: string } | null
  hasCommand(commandId: string): boolean
}

/** Options every shell environment takes; an engine adds its own. */
export interface ShellEnvironmentOptions {
  shellIdFactory?: () => string
  commandIdFactory?: () => string
  initialEnv?: Record<string, string>
  /** Default per-exec output view budget (bytes). */
  maxOutputBytes?: number
  /** Ceiling for a raw-byte final stream carried in memory. */
  maxBinaryBytes?: number
  /** Ceiling for a single command's in-memory output capture. */
  maxCaptureBytes?: number
}

// Fallback observation window for direct exec() calls that omit timeoutMs (internal instant
// commands like demi/todo). The model-facing shell_exec tool requires timeoutMs, so the model
// controls it per call — there is intentionally no configurable global default.
export const DEFAULT_TIMEOUT_MS = 10_000
export const DEFAULT_OUTPUT_LIMIT_BYTES = 1024 * 1024
/**
 * Ceiling for a raw-byte final stream. Sized so an ordinary viewing-grade clip
 * survives the shell and reaches the layer that decides what to do with it,
 * while still bounding a runaway producer.
 */
export const DEFAULT_BINARY_LIMIT_BYTES = 16 * 1024 * 1024
/**
 * Ceiling for a single command's in-memory output capture. Sized to the same
 * order as the in-shell file read limit: the execution model buffers whole
 * command outputs (in several copies), so this is a hard memory-safety bound
 * on the embedding process, not a view budget.
 */
export const DEFAULT_CAPTURE_LIMIT_BYTES = 64 * 1024 * 1024
/** Upper bound for a single exec observation window. */
export const MAX_TIMEOUT_MS = 600_000

export function normalizeTimeoutMs(value: number): number {
  if (!Number.isFinite(value) || value < 1 || value > MAX_TIMEOUT_MS) {
    throw new Error(`timeoutMs must be between 1 and ${MAX_TIMEOUT_MS}`)
  }
  return Math.floor(value)
}

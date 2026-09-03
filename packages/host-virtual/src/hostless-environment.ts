// The hostless shell environment (`commands.md` § Hostless): tinybash over
// the hostless Host, root commands through an injected dispatcher, no
// process anywhere. This is where Demi's Host contract and command ABI
// meet tinybash's own system interface; tinybash itself depends on
// neither. Beside `VirtualHost` the way `RemoteShellEnvironment` sits
// beside `RemoteHost`: one execution target, its Host and its shell.
import {
  DEFAULT_BINARY_LIMIT_BYTES,
  DEFAULT_CAPTURE_LIMIT_BYTES,
  DEFAULT_OUTPUT_LIMIT_BYTES,
  DEFAULT_TIMEOUT_MS,
  appendRecordOutput,
  commandStatusView,
  createCommandRecord,
  finalStdoutBoundary,
  normalizeTimeoutMs,
  settleExited,
  type Host,
  type ShellAbortInput,
  type ShellCommandRecord,
  type ShellCommandStatus,
  type ShellEnvironment,
  type ShellEnvironmentOptions,
  type ShellExecInput,
  type ShellStatusInput,
  type ShellWriteInput,
} from '@demicodes/shell'
import { parseTinybash, runTinybash, type DispatchIO, type RootPaths, type ShellState, type TinybashOutside } from '@demicodes/tinybash'
import { ByteQueue, concatBytes, delay, errorMessage, isAbsolutePath, toBytes } from '@demicodes/utils'

export interface HostlessEnvironmentOptions extends ShellEnvironmentOptions {
  /** The conversation's store-backed Host. */
  host: Host
  /** Root names → the path-typed arguments of an invocation, from the trees' path marks (`rootPaths` in the loader). */
  roots: ReadonlyMap<string, RootPaths>
  /** Runs a root command; `argv` excludes the root name. */
  dispatch: (root: string, argv: string[], io: DispatchIO) => Promise<number>
  /** `$HOME`, and the first namespace prefix. */
  home: string
  /** Absolute prefixes a script may touch (`sessions-and-targets.md` § The namespace). */
  namespace: readonly string[]
  /** Owner names `ls -l` shows. */
  identity: { user: string; group: string }
}

interface HostlessShell {
  id: string
  agentSessionId: string | null
  commandStorageId: string
  state: ShellState
  foreground?: RunningCommand
  exited: boolean
}

interface RunningCommand {
  record: ShellCommandRecord
  controller: AbortController
  stdin: ByteQueue
  settled: Promise<void>
}

/** The variables `createShell` sets on every shell; a session's own are the rest. */
const STARTING_VARS = new Set(['HOME', 'USER', 'DEMI_SHELL_ID', 'DEMI_SESSION_ID'])

/** How long `abort` waits for the statement in flight to honour the signal. */
const ABORT_GRACE_MS = 2_000

/**
 * The shell behind the `shell_*` tools for a hostless conversation
 * (`commands.md` § Hostless): tinybash over the conversation's Host, root
 * commands through the loader, no process anywhere. Command records, views
 * and artifacts are the ones every shell environment shares, so the tools
 * cannot tell which engine ran the script.
 */
export class HostlessEnvironment implements ShellEnvironment {
  private readonly host: Host
  private readonly roots: ReadonlyMap<string, RootPaths>
  private readonly dispatch: (root: string, argv: string[], io: DispatchIO) => Promise<number>
  private readonly home: string
  private readonly namespace: readonly string[]
  private readonly identity: { user: string; group: string }
  private readonly shellIdFactory: () => string
  private readonly commandIdFactory: () => string
  private readonly initialEnv: Record<string, string>
  private readonly defaultOutputLimitBytes: number
  private readonly binaryLimitBytes: number
  private readonly captureLimitBytes: number
  private readonly shells = new Map<string, HostlessShell>()
  private readonly defaultShellByAgentSessionId = new Map<string, string>()
  private readonly commandsById = new Map<string, ShellCommandRecord>()
  private readonly runningById = new Map<string, RunningCommand>()

  constructor(options: HostlessEnvironmentOptions) {
    this.host = options.host
    this.roots = options.roots
    this.dispatch = options.dispatch
    this.home = options.home
    this.namespace = options.namespace
    this.identity = options.identity
    this.shellIdFactory = options.shellIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.commandIdFactory = options.commandIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.initialEnv = options.initialEnv ?? {}
    this.defaultOutputLimitBytes = options.maxOutputBytes ?? DEFAULT_OUTPUT_LIMIT_BYTES
    this.binaryLimitBytes = options.maxBinaryBytes ?? DEFAULT_BINARY_LIMIT_BYTES
    this.captureLimitBytes = options.maxCaptureBytes ?? DEFAULT_CAPTURE_LIMIT_BYTES
  }

  getShell(shellId: string): { id: string } | null {
    return this.shells.get(shellId) ?? null
  }

  hasCommand(commandId: string): boolean {
    return this.commandsById.has(commandId)
  }

  /**
   * The parse-first decision for an exec, before anything runs
   * (`sessions-and-targets.md` § The upgrade condition): null when the
   * script is inside tinybash's subset under the state of the shell the
   * exec would use, otherwise why it is outside. The embedder hands an
   * outside script to a machine.
   */
  async outside(input: ShellExecInput): Promise<TinybashOutside | null> {
    const parsed = await parseTinybash(input.script, this.roots, this.namespace, this.stateFor(input), this.host.fs)
    return parsed.kind === 'outside' ? parsed : null
  }

  /**
   * What a machine's shell must be told to continue where this exec's shell
   * stands: the working directory, and the variables the session set beyond
   * the ones every shell starts with.
   */
  handoverOf(input: ShellExecInput): { cwd: string; vars: Record<string, string> } {
    const state = this.stateFor(input)
    const vars: Record<string, string> = {}
    for (const [key, value] of Object.entries(state.vars)) {
      if (!(key in this.initialEnv) && !STARTING_VARS.has(key)) vars[key] = value
    }
    return { cwd: state.cwd, vars }
  }

  /** The shell state an exec would start from: the named shell's, the session default's, or a fresh one for an ephemeral exec. */
  private stateFor(input: ShellExecInput): ShellState {
    if (input.shellId) return this.requireShell(input.shellId).state
    if (input.ephemeral) return this.initialState(undefined, input.cwd, '')
    return this.defaultShell(input.agentSessionId).state
  }

  async exec(input: ShellExecInput): Promise<ShellCommandStatus> {
    const timeoutMs = normalizeTimeoutMs(input.timeoutMs ?? DEFAULT_TIMEOUT_MS)
    if (input.shellId && input.ephemeral) {
      throw new Error('ShellExecInput: "shellId" and "ephemeral" are mutually exclusive')
    }
    if (input.cwd !== undefined && !input.ephemeral) {
      throw new Error('ShellExecInput: "cwd" requires "ephemeral"; a persistent shell owns its cwd')
    }
    if (input.cwd !== undefined) {
      if (!isAbsolutePath(input.cwd) || !this.namespace.some((prefix) => input.cwd === prefix || input.cwd!.startsWith(`${prefix}/`))) {
        throw new Error(`Shell exec cwd is outside the hostless namespace: ${input.cwd}`)
      }
      const stat = await this.host.fs.stat(input.cwd).catch(() => null)
      if (!stat?.isDirectory) throw new Error(`Shell exec cwd is not a directory: ${input.cwd}`)
    }
    const shell = input.shellId
      ? this.requireShell(input.shellId)
      : input.ephemeral
        ? this.createShell(input.agentSessionId, input.cwd)
        : this.defaultShell(input.agentSessionId)
    if (shell.exited) throw new Error(`Shell session "${shell.id}" has exited`)
    if (shell.foreground) {
      throw new Error(`Shell session "${shell.id}" is already running command "${shell.foreground.record.id}"`)
    }
    const running = this.start(shell, input.script, input.signal)
    await settledOrElapsed(running.settled, timeoutMs)
    return this.view(running.record)
  }

  async status(input: ShellStatusInput): Promise<ShellCommandStatus> {
    return this.view(this.requireCommand(input.commandId))
  }

  async write(input: ShellWriteInput): Promise<ShellCommandStatus> {
    const record = this.requireCommand(input.commandId)
    if (record.status !== 'running') throw new Error(`Command "${record.id}" is not running`)
    const running = this.runningById.get(record.id)
    if (!running) throw new Error(`Command "${record.id}" is not running`)
    const data = toBytes(input.stdin)
    if (data.byteLength === 0) throw new Error('shell_write field "stdin" must not be empty; use shell_status to poll')
    running.stdin.push(data)
    return this.view(record)
  }

  async abort(input: ShellAbortInput): Promise<ShellCommandStatus> {
    const record = this.requireCommand(input.commandId)
    const running = this.runningById.get(record.id)
    if (record.status !== 'running' || !running) return this.view(record)
    running.controller.abort()
    running.stdin.close()
    await settledOrElapsed(running.settled, ABORT_GRACE_MS)
    if (record.status === 'running') this.markAborted(running)
    return this.view(record)
  }

  async releaseCommand(commandId: string): Promise<boolean> {
    const record = this.commandsById.get(commandId)
    if (!record) return false
    if (record.status === 'running') await this.abort({ commandId })
    this.commandsById.delete(commandId)
    return true
  }

  async disposeShell(shellId: string): Promise<boolean> {
    const shell = this.shells.get(shellId)
    if (!shell) return false
    if (shell.foreground) await this.abort({ commandId: shell.foreground.record.id })
    shell.exited = true
    this.shells.delete(shellId)
    for (const [agentSessionId, id] of this.defaultShellByAgentSessionId) {
      if (id === shellId) this.defaultShellByAgentSessionId.delete(agentSessionId)
    }
    return true
  }

  async disposeAllShells(): Promise<void> {
    for (const shellId of [...this.shells.keys()]) await this.disposeShell(shellId)
  }

  private start(shell: HostlessShell, script: string, callerSignal: AbortSignal | undefined): RunningCommand {
    const id = this.commandIdFactory()
    const record = createCommandRecord({
      id,
      shellId: shell.id,
      commandStorageId: shell.commandStorageId,
      script,
      outputLimitBytes: this.defaultOutputLimitBytes,
    })
    this.commandsById.set(id, record)
    const controller = new AbortController()
    if (callerSignal) {
      if (callerSignal.aborted) controller.abort()
      else callerSignal.addEventListener('abort', () => controller.abort(), { once: true })
    }
    const stdin = new ByteQueue()
    const stdoutBytes: Uint8Array[] = []
    const stdoutDecoder = new TextDecoder()
    const stderrDecoder = new TextDecoder()
    let captured = 0
    let overflowed = false
    const ingest = (stream: 'stdout' | 'stderr', data: string | Uint8Array): void => {
      if (overflowed) return
      const bytes = toBytes(data)
      captured += bytes.byteLength
      if (captured > this.captureLimitBytes) {
        overflowed = true
        controller.abort()
        return
      }
      if (stream === 'stdout') stdoutBytes.push(bytes)
      const text = (stream === 'stdout' ? stdoutDecoder : stderrDecoder).decode(bytes, { stream: true })
      if (stream === 'stdout') record.stdout += text
      else record.stderr += text
      appendRecordOutput(record, stream, text)
      record.lastOutputAt = Date.now()
    }
    const run = runTinybash({
      script,
      roots: this.roots,
      namespace: this.namespace,
      dispatch: async (root, argv, io) => {
        const exitCode = await this.dispatch(root, argv, io)
        return exitCode
      },
      fs: this.host.fs,
      state: shell.state,
      io: { stdout: (data) => ingest('stdout', data), stderr: (data) => ingest('stderr', data) },
      identity: this.identity,
      stdin: stdin.stream(),
      signal: controller.signal,
    })
    const running: RunningCommand = { record, controller, stdin, settled: Promise.resolve() }
    running.settled = run
      .then(
        (result) => {
          if (overflowed) {
            const message = `hostless shell: output exceeded the ${this.captureLimitBytes}-byte capture limit and the command was stopped; narrow the output at the source (filters, head, tighter paths)\n`
            return this.finish(running, 137, stdoutBytes, `${record.stderr}${message}`)
          }
          if (result.kind === 'outside') return this.finish(running, 2, stdoutBytes, `${record.stderr}${result.message}\n`)
          if (controller.signal.aborted && result.exitCode === 130) return this.markAborted(running)
          return this.finish(running, result.exitCode, stdoutBytes, record.stderr)
        },
        (error: unknown) => this.finish(running, 1, stdoutBytes, `${record.stderr}hostless shell: ${errorMessage(error)}\n`),
      )
      .finally(() => {
        stdin.close()
        this.runningById.delete(id)
        if (shell.foreground === running) shell.foreground = undefined
      })
    shell.foreground = running
    this.runningById.set(id, running)
    return running
  }

  private finish(running: RunningCommand, exitCode: number, stdoutBytes: Uint8Array[], stderr: string): void {
    const { record } = running
    if (record.status !== 'running') return
    const bytes = concatBytes(stdoutBytes)
    const boundary = finalStdoutBoundary(bytes, this.binaryLimitBytes)
    settleExited(record, exitCode, boundary.text, stderr, boundary.binary)
  }

  private markAborted(running: RunningCommand): void {
    const { record } = running
    if (record.status !== 'running') return
    record.lastOutputAt = Date.now()
    record.status = 'aborted'
  }

  private view(record: ShellCommandRecord): ShellCommandStatus {
    return commandStatusView(record, this.defaultOutputLimitBytes)
  }

  private requireShell(shellId: string): HostlessShell {
    const shell = this.shells.get(shellId)
    if (!shell) throw new Error(`Unknown shell session "${shellId}"`)
    return shell
  }

  private requireCommand(commandId: string): ShellCommandRecord {
    const record = this.commandsById.get(commandId)
    if (!record) throw new Error(`Unknown command "${commandId}"`)
    return record
  }

  private defaultShell(agentSessionId: string | undefined): HostlessShell {
    const key = agentSessionId ?? ''
    const existing = this.defaultShellByAgentSessionId.get(key)
    if (existing) {
      const shell = this.shells.get(existing)
      // A busy default shell is left to its command: the session gets a fresh
      // shell for this exec, so a long-running command never blocks the next.
      if (shell && !shell.exited) return shell.foreground && agentSessionId ? this.createShell(agentSessionId) : shell
    }
    const shell = this.createShell(agentSessionId)
    this.defaultShellByAgentSessionId.set(key, shell.id)
    return shell
  }

  private initialState(agentSessionId: string | undefined, initialCwd: string | undefined, shellId: string): ShellState {
    const vars: Record<string, string> = { ...this.initialEnv, HOME: this.home, USER: this.identity.user, DEMI_SHELL_ID: shellId }
    if (agentSessionId) vars.DEMI_SESSION_ID = agentSessionId
    return { cwd: initialCwd ?? this.host.defaultCwd, home: this.home, vars }
  }

  private createShell(agentSessionId: string | undefined, initialCwd?: string): HostlessShell {
    const id = this.shellIdFactory()
    const shell: HostlessShell = {
      id,
      agentSessionId: agentSessionId ?? null,
      commandStorageId: agentSessionId ?? id,
      state: this.initialState(agentSessionId, initialCwd, id),
      exited: false,
    }
    this.shells.set(id, shell)
    return shell
  }
}

/** Waits for the command or the deadline, whichever comes first; a won race leaves no timer behind. */
async function settledOrElapsed(settled: Promise<unknown>, ms: number): Promise<void> {
  const timer = new AbortController()
  try {
    await Promise.race([settled, delay(ms, timer.signal)])
  } finally {
    timer.abort()
  }
}

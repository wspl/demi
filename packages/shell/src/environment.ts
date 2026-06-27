import { decodeUtf8, encodeUtf8, tail } from '@demicodes/utils'
import { ArithmeticError, BadSubstitutionError, ExitError, ExecutionLimitError, Interpreter, type InterpreterContext, type InterpreterState } from '@demicodes/just-bash/interpreter'
import type { HostSpawnRedirection } from '@demicodes/just-bash/interpreter'
import type { ScriptNode } from '@demicodes/just-bash/ast/types'
import { createLazyCommands, type CommandName } from '@demicodes/just-bash/commands'
import { decodeBytesToUtf8 } from '@demicodes/just-bash/encoding'
import { parse } from '@demicodes/just-bash/parser'
import { ParseException } from '@demicodes/just-bash/parser/types'
import { LexerError } from '@demicodes/just-bash/parser/lexer'
import type { Command as ForkCommand, CommandRegistry as ForkCommandRegistry, ExecResult as ForkExecResult, IFileSystem } from '@demicodes/just-bash/types'
import { resolveLimits } from '@demicodes/just-bash/limits'
import { CommandRegistry, type CommandSpec } from './command'
import { extractSimpleBackgroundCommand, formatCommandDisplay } from './background-command'
import {
  buildBashopts,
  buildShellopts,
  createOutputSinks,
  flushForegroundSinks,
  notifyForegroundWaiters,
  pumpOutputStream,
  pumpStream,
  recordForegroundChunk,
  snapshotFromAccumulator,
  snapshotFromForeground,
} from './environment-output'
import type { BackgroundJob, BoundaryOutcome, ForegroundProcess, ShellSession } from './environment-state'
import type { Host } from './host'
import { HostBackedFileSystem, virtualDirectory, virtualFile, type VirtualFileSystemNode } from './host-fs'
import { AgentSessionCommandStorage } from './storage'
import { CommandArtifactStore } from './command-artifact-store'
import { commandSpecToForkCommand } from './registered-command-adapter'

export interface BashEnvironmentOptions {
  host: Host
  commands?: CommandRegistry
  shellIdFactory?: () => string
  commandIdFactory?: () => string
  initialEnv?: Record<string, string>
  maxOutputBytes?: number
}

export interface ShellExecInput {
  script: string
  shellId?: string
  agentSessionId?: string
  timeoutMs?: number
  maxOutputBytes?: number
  signal?: AbortSignal
}

export interface ShellStatusInput {
  commandId: string
  stdoutOffset?: number
  stderrOffset?: number
  outputOffset?: number
  maxOutputBytes?: number
}

export interface ShellWriteInput {
  commandId: string
  stdin: string | Uint8Array
  maxOutputBytes?: number
  signal?: AbortSignal
}

export interface ShellAbortInput {
  commandId: string
  maxOutputBytes?: number
}

export interface OutputSnapshot {
  stdoutDelta: string
  stderrDelta: string
  stdoutTail: string
  stderrTail: string
  totalStdoutBytes: number
  totalStderrBytes: number
  truncated: boolean
}

export interface StreamArtifact {
  path: string
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

export interface ShellOutputArtifact {
  path: string
  offset: number
  text: string
  tail: string
  chunks: ShellOutputChunk[]
  bytes: number
  truncated: boolean
}

export type BashAuditEvent =
  | { kind: 'registered-command'; name: string; args: string[]; exitCode: number }
  | { kind: 'portable-command'; name: string; args: string[]; cwd: string; exitCode: number }
  | { kind: 'system-command'; name: string; args: string[]; cwd: string; exitCode: number | null }

export interface CommandMetadataRecord {
  kind: 'registered-command'
  name: string
  args: string[]
  metadata: unknown
}

export type ShellCommandSnapshot =
  | {
      status: 'exited'
      shellId: string
      commandId: string
      exitCode: number
      stdout: StreamArtifact
      stderr: StreamArtifact
      output: ShellOutputArtifact
      runningMs: number
      idleMs: number
      audit: BashAuditEvent[]
      commandMetadata?: CommandMetadataRecord[]
    }
  | {
      status: 'running'
      shellId: string
      commandId: string
      stdout: StreamArtifact
      stderr: StreamArtifact
      output: ShellOutputArtifact
      runningMs: number
      idleMs: number
    }
  | {
      status: 'aborted'
      shellId: string
      commandId: string
      stdout: StreamArtifact
      stderr: StreamArtifact
      output: ShellOutputArtifact
      runningMs: number
      idleMs: number
    }

interface ShellCommandRecord {
  id: string
  shellId: string
  commandScopeId: string
  script: string
  startedAt: number
  lastOutputAt: number
  status: 'running' | 'exited' | 'aborted'
  stdout: string
  stderr: string
  stdoutOffset: number
  stderrOffset: number
  outputChunks: ShellOutputRecordChunk[]
  outputOffset: number
  exitCode?: number
  audit: BashAuditEvent[]
  commandMetadata: CommandMetadataRecord[]
}

interface PersistedShellCommandArtifact {
  status: 'running' | 'exited' | 'aborted'
  shellId: string
  commandId: string
  startedAt: number
  lastOutputAt: number
  exitCode: number | null
  stdout: string
  stderr: string
}

// Fallback observation window for direct exec() calls that omit timeoutMs (internal instant
// commands like editor/todo). The model-facing shell_exec tool requires timeoutMs, so the model
// controls it per call — there is intentionally no configurable global default.
const DEFAULT_TIMEOUT_MS = 10_000
const DEFAULT_OUTPUT_LIMIT_BYTES = 1024 * 1024
const MAX_TIMEOUT_MS = 600_000
const DEMI_PORTABLE_COMMANDS: CommandName[] = [
  'cat',
  'ls',
  'mkdir',
  'rmdir',
  'touch',
  'rm',
  'cp',
  'mv',
  'ln',
  'chmod',
  'readlink',
  'head',
  'tail',
  'wc',
  'stat',
  'grep',
  'fgrep',
  'egrep',
  'rg',
  'sed',
  'awk',
  'sort',
  'uniq',
  'comm',
  'cut',
  'paste',
  'tr',
  'rev',
  'nl',
  'fold',
  'expand',
  'unexpand',
  'strings',
  'column',
  'join',
  'tee',
  'find',
  'basename',
  'dirname',
  'tree',
  'du',
  'jq',
  'base64',
  'diff',
  'seq',
  'expr',
  'md5sum',
  'sha1sum',
  'sha256sum',
  'file',
  'tac',
  'od',
]
export class BashEnvironment {
  private readonly host: Host
  private readonly commands: CommandRegistry
  private readonly shellIdFactory: () => string
  private readonly commandIdFactory: () => string
  private readonly initialEnv: Record<string, string>
  private readonly defaultOutputLimitBytes: number
  private readonly shells = new Map<string, ShellSession>()
  private readonly defaultShellByAgentSessionId = new Map<string, string>()
  private readonly commandsById = new Map<string, ShellCommandRecord>()
  private readonly artifacts: CommandArtifactStore

  constructor(options: BashEnvironmentOptions) {
    this.host = options.host
    this.artifacts = new CommandArtifactStore(this.host.store)
    this.commands = options.commands ?? new CommandRegistry()
    this.shellIdFactory = options.shellIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.commandIdFactory = options.commandIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.initialEnv = options.initialEnv ?? {}
    this.defaultOutputLimitBytes = options.maxOutputBytes ?? DEFAULT_OUTPUT_LIMIT_BYTES
  }

  getShell(shellId: string): ShellSession | null {
    return this.shells.get(shellId) ?? null
  }

  registerCommand(spec: CommandSpec): void {
    if (this.commands.get(spec.name)) return
    this.commands.register(spec)
  }

  registeredCommands(): CommandSpec[] {
    return this.commands.list()
  }

  async exec(input: ShellExecInput): Promise<ShellCommandSnapshot> {
    const timeoutMs = normalizeTimeoutMs(input.timeoutMs ?? DEFAULT_TIMEOUT_MS)
    const session = input.shellId ? this.requireShell(input.shellId) : this.availableDefaultShell(input.agentSessionId)
    if (session.exited) throw new Error(`Shell session "${session.id}" has exited`)
    if (session.pendingExec || session.foreground) {
      const commandId = session.activeCommandId ?? session.foreground?.commandId ?? 'unknown'
      throw new Error(`Shell session "${session.id}" is already running command "${commandId}"`)
    }

    return this.runScript(session, input.script, { ...input, timeoutMs })
  }

  async status(input: ShellStatusInput): Promise<ShellCommandSnapshot> {
    const record = this.requireCommand(input.commandId)
    return this.snapshotCommand(record, input)
  }

  async write(input: ShellWriteInput): Promise<ShellCommandSnapshot> {
    const record = this.requireCommand(input.commandId)
    if (record.status !== 'running') throw new Error(`Command "${record.id}" is not running`)
    const session = this.requireShell(record.shellId)
    const foreground = this.requireForegroundCommand(session, record.id)
    const data = typeof input.stdin === 'string' ? encodeUtf8(input.stdin) : input.stdin
    if (data.byteLength === 0) throw new Error('shell_write field "stdin" must not be empty; use shell_status to poll')
    await foreground.handle.writeStdin(data)
    return this.snapshotCommand(record, input)
  }

  async abort(input: ShellAbortInput): Promise<ShellCommandSnapshot> {
    const record = this.requireCommand(input.commandId)
    if (record.status !== 'running') return this.snapshotCommand(record, input)
    const session = this.requireShell(record.shellId)
    const foreground = this.requireForegroundCommand(session, record.id)
    foreground.abortController.abort()
    await foreground.handle.kill('SIGTERM')
    session.state.lastExitCode = 130
    return this.collectAborted(session, record, foreground, input)
  }

  async releaseCommand(commandId: string): Promise<boolean> {
    const record = this.commandsById.get(commandId)
    if (!record || record.status === 'running') return false
    this.commandsById.delete(commandId)
    await this.artifacts.release(record.commandScopeId, commandId)
    return true
  }

  async disposeShell(shellId: string): Promise<boolean> {
    const session = this.shells.get(shellId)
    if (!session) return false
    await this.killShell(session)
    this.shells.delete(shellId)
    for (const [agentSessionId, defaultShellId] of this.defaultShellByAgentSessionId) {
      if (defaultShellId === shellId) this.defaultShellByAgentSessionId.delete(agentSessionId)
    }
    return true
  }

  async disposeAllShells(): Promise<void> {
    for (const shellId of this.shells.keys()) {
      await this.disposeShell(shellId)
    }
  }

  private async killShell(session: ShellSession): Promise<void> {
    const foreground = session.foreground
    session.foreground = undefined
    session.pendingExec = undefined
    if (foreground) {
      foreground.abortController.abort()
      await foreground.handle.kill('SIGKILL').catch(() => {})
      await foreground.exitPromise.catch(() => {})
    }
    for (const job of session.backgroundJobs.values()) {
      await job.handle.kill('SIGKILL').catch(() => {})
      await job.exitPromise.catch(() => {})
    }
    session.backgroundJobs.clear()
    if (session.abortController) session.abortController.abort()
  }

  private requireShell(shellId: string): ShellSession {
    const session = this.shells.get(shellId)
    if (!session) throw new Error(`Unknown shell session "${shellId}"`)
    return session
  }

  private requireCommand(commandId: string): ShellCommandRecord {
    const record = this.commandsById.get(commandId)
    if (!record) throw new Error(`Unknown shell command "${commandId}"`)
    const session = this.shells.get(record.shellId)
    const foreground = session?.foreground
    if (record.status === 'running' && foreground?.commandId === record.id) {
      record.stdout = foreground.stdoutBuffer
      record.stderr = foreground.stderrBuffer
      record.outputChunks = [...foreground.outputChunks]
      record.lastOutputAt = foreground.lastOutputAt
    }
    return record
  }

  private requireForegroundCommand(session: ShellSession, commandId: string): ForegroundProcess {
    const foreground = session.foreground
    if (!foreground || foreground.commandId !== commandId) {
      throw new Error(`Command "${commandId}" has no foreground process`)
    }
    return foreground
  }

  private defaultShell(agentSessionId: string | undefined): ShellSession {
    if (!agentSessionId) return this.createShell(undefined)
    const existingShellId = this.defaultShellByAgentSessionId.get(agentSessionId)
    const existing = existingShellId ? this.shells.get(existingShellId) : undefined
    if (existing && !existing.exited) return existing
    const shell = this.createShell(agentSessionId)
    this.defaultShellByAgentSessionId.set(agentSessionId, shell.id)
    return shell
  }

  private availableDefaultShell(agentSessionId: string | undefined): ShellSession {
    const shell = this.defaultShell(agentSessionId)
    if (!agentSessionId || shell.exited || (!shell.pendingExec && !shell.foreground)) return shell
    return this.createShell(agentSessionId)
  }

  private createShell(agentSessionId: string | undefined): ShellSession {
    const id = this.shellIdFactory()
    const commandScopeId = agentSessionId ?? id
    const cwd = this.host.defaultCwd
    const fs = new HostBackedFileSystem(this.host, {
      lookup: (path) => this.lookupVirtualArtifact(commandScopeId, path),
    })
    const env = new Map<string, string>()
    for (const [key, value] of Object.entries(this.initialEnv)) env.set(key, value)
    env.set('PWD', cwd)
    env.set('DEMI_SESSION_ID', commandScopeId)
    env.set('DEMI_SHELL_ID', id)
    if (!env.has('IFS')) env.set('IFS', ' \t\n')
    if (!env.has('PS1')) env.set('PS1', '')
    if (!env.has('PS2')) env.set('PS2', '> ')
    if (!env.has('SHLVL')) env.set('SHLVL', '1')
    const exportedVars = new Set<string>(['PWD', 'DEMI_SESSION_ID', 'DEMI_SHELL_ID'])
    for (const key of env.keys()) {
      if (key !== key.toLowerCase()) exportedVars.add(key)
    }
    for (const key of Object.keys(this.initialEnv)) exportedVars.add(key)

    const state: InterpreterState = {
      env,
      cwd,
      previousDir: cwd,
      functions: new Map(),
      localScopes: [],
      callDepth: 0,
      sourceDepth: 0,
      commandCount: 0,
      lastExitCode: 0,
      lastArg: '',
      startTime: Date.now(),
      lastBackgroundPid: 0,
      virtualPid: 1,
      virtualPpid: 0,
      virtualUid: 1000,
      virtualGid: 1000,
      bashPid: 1,
      nextVirtualPid: 2,
      currentLine: 1,
      options: {
        errexit: false,
        pipefail: false,
        nounset: false,
        xtrace: false,
        verbose: false,
        posix: false,
        allexport: false,
        noclobber: false,
        noglob: false,
        noexec: false,
        vi: false,
        emacs: false,
      },
      shoptOptions: {
        extglob: false,
        dotglob: false,
        nullglob: false,
        failglob: false,
        globstar: false,
        globskipdots: true,
        nocaseglob: false,
        nocasematch: false,
        expand_aliases: false,
        lastpipe: false,
        xpg_echo: false,
      },
      inCondition: false,
      loopDepth: 0,
      exportedVars,
      readonlyVars: new Set(['SHELLOPTS', 'BASHOPTS']),
      hashTable: new Map(),
    }
    state.env.set('SHELLOPTS', buildShellopts(state.options))
    state.env.set('BASHOPTS', buildBashopts(state.shoptOptions))

    const forkCommands: ForkCommandRegistry = new Map()
    const session: ShellSession = {
      id,
      commandScopeId,
      state,
      fs,
      interpreter: undefined as unknown as Interpreter,
      forkCommands,
      accumulator: { stdout: '', stderr: '', audit: [], commandMetadata: [] },
      startStdoutBytes: 0,
      startStderrBytes: 0,
      totalStdoutBytes: 0,
      totalStderrBytes: 0,
      stdoutTail: '',
      stderrTail: '',
      truncated: false,
      foregroundWaiters: new Set(),
      backgroundJobs: new Map(),
      nextBackgroundJobId: 1,
      exited: false,
    }
    for (const command of createPortableCommands(session)) {
      forkCommands.set(command.name, command)
    }
    const storage = new AgentSessionCommandStorage(this.host.store, commandScopeId)
    for (const spec of this.commands.list()) {
      forkCommands.set(spec.name, commandSpecToForkCommand(session, spec, storage))
    }
    const abortController = new AbortController()
    session.abortController = abortController
    const limits = resolveLimits({ maxOutputSize: 1024 * 1024 * 1024, maxCommandCount: 1_000_000, maxLoopIterations: 1_000_000, maxCallDepth: 1000, maxGlobOperations: 1_000_000 })
    const interpreter = new Interpreter(
      {
        fs: fs as IFileSystem,
        commands: forkCommands,
        limits,
        exec: async () => ({ stdout: '', stderr: '', exitCode: 0 }),
        hostSpawn: (command, args, opts) => this.hostSpawn(session, command, args, opts),
        rejectTimedPipelines: true,
        jobControl: {
          startBackground: (statement) => this.startBackgroundJob(session, statement),
          jobs: (args) => this.listBackgroundJobs(session, args),
          wait: (args) => this.waitForBackgroundJob(session, args),
        },
      },
      state,
    )
    session.interpreter = interpreter
    this.shells.set(id, session)
    return session
  }

  private async runScript(
    session: ShellSession,
    script: string,
    input: ShellExecInput & { timeoutMs: number },
  ): Promise<ShellCommandSnapshot> {
    const record = this.createCommandRecord(session, script)
    let ast: ScriptNode
    try {
      ast = parse(script)
    } catch (error) {
      if (error instanceof ParseException || error instanceof LexerError) {
        const message = (error as Error).message
        record.stderr = `bash: ${message}\n`
        appendRecordOutput(record, 'stderr', record.stderr)
        record.status = 'exited'
        record.exitCode = 2
        session.state.lastExitCode = 2
        session.activeCommandId = undefined
        return this.snapshotCommand(record, input)
      }
      throw error
    }
    session.accumulator = { stdout: '', stderr: '', audit: [], commandMetadata: [] }
    session.startStdoutBytes = session.totalStdoutBytes
    session.startStderrBytes = session.totalStderrBytes
    session.abortController = new AbortController()
    session.activeCommandId = record.id

    const execPromise = session.interpreter.executeScript(ast).then(
      (result) => result,
      (error) => error as Error,
    )
    session.pendingExec = execPromise
    execPromise.then(
      (result) => {
        try {
          if (record.status === 'running' && session.pendingExec === execPromise) {
            this.collectExited(session, record, result, session.foreground, {
              stdoutOffset: record.stdoutOffset,
              stderrOffset: record.stderrOffset,
              outputOffset: record.outputOffset,
            })
          }
        } catch {
          // The foreground caller observes execution errors; this background settle path
          // exists only to make later shell_status calls see naturally completed commands.
        }
      },
      () => {},
    )
    return this.raceForeground(session, record, undefined, execPromise, input)
  }

  private createCommandRecord(session: ShellSession, script: string): ShellCommandRecord {
    const id = this.commandIdFactory()
    const now = Date.now()
    const record: ShellCommandRecord = {
      id,
      shellId: session.id,
      commandScopeId: session.commandScopeId,
      script,
      startedAt: now,
      lastOutputAt: now,
      status: 'running',
      stdout: '',
      stderr: '',
      stdoutOffset: 0,
      stderrOffset: 0,
      outputChunks: [],
      outputOffset: 0,
      audit: [],
      commandMetadata: [],
    }
    this.commandsById.set(id, record)
    return record
  }

  private async startBackgroundJob(session: ShellSession, statement: unknown): Promise<ForkExecResult | null> {
    const backgroundCommand = extractSimpleBackgroundCommand(statement)
    if (!backgroundCommand) return null

    const id = session.nextBackgroundJobId++
    const handle = await this.host.process.spawn({
      command: backgroundCommand.command,
      args: backgroundCommand.args,
      cwd: session.state.cwd,
      env: this.exportedEnv(session),
      killProcessGroup: true,
    })
    await handle.closeStdin().catch(() => {})

    const job: BackgroundJob = {
      id,
      command: backgroundCommand.command,
      args: backgroundCommand.args,
      display: formatCommandDisplay(backgroundCommand.command, backgroundCommand.args),
      cwd: session.state.cwd,
      handle,
      stdoutBuffer: '',
      stderrBuffer: '',
      stdoutPump: Promise.resolve(),
      stderrPump: Promise.resolve(),
      exitPromise: handle.wait(),
    }
    job.stdoutPump = pumpStream(handle.stdout, (chunk) => {
      job.stdoutBuffer += decodeUtf8(chunk)
    })
    job.stderrPump = pumpStream(handle.stderr, (chunk) => {
      job.stderrBuffer += decodeUtf8(chunk)
    })
    session.backgroundJobs.set(id, job)
    session.state.lastBackgroundPid = id
    session.state.env.set('!', String(id))
    return { stdout: `[${id}] ${job.display}\n`, stderr: '', exitCode: 0 }
  }

  private async listBackgroundJobs(session: ShellSession, args: string[]): Promise<ForkExecResult> {
    if (args.length > 0) {
      return { stdout: '', stderr: `bash: jobs: unsupported option or argument: ${args.join(' ')}\n`, exitCode: 2 }
    }
    let stdout = ''
    for (const job of session.backgroundJobs.values()) {
      stdout += `[${job.id}] Running ${job.display}\n`
    }
    return { stdout, stderr: '', exitCode: 0 }
  }

  private async waitForBackgroundJob(session: ShellSession, args: string[]): Promise<ForkExecResult> {
    if (args.length !== 1) {
      return { stdout: '', stderr: 'bash: wait: expected a single job spec\n', exitCode: 2 }
    }
    const match = args[0].match(/^%(\d+)$/)
    if (!match) {
      return { stdout: '', stderr: `bash: wait: ${args[0]}: unsupported job spec\n`, exitCode: 2 }
    }
    const id = Number.parseInt(match[1], 10)
    const job = session.backgroundJobs.get(id)
    if (!job) {
      return { stdout: '', stderr: `bash: wait: %${id}: no such job\n`, exitCode: 127 }
    }

    const exit = await job.exitPromise
    await Promise.allSettled([job.stdoutPump, job.stderrPump])
    session.backgroundJobs.delete(id)

    const exitCode = exit.exitCode ?? 127
    const stderr =
      exit.exitCode === null && job.stderrBuffer.length === 0
        ? `${job.command}: ${exit.signal ?? 'command not found'}\n`
        : job.stderrBuffer
    session.accumulator.audit.push({ kind: 'system-command', name: job.command, args: job.args, cwd: job.cwd, exitCode })
    return { stdout: job.stdoutBuffer, stderr, exitCode }
  }

  private exportedEnv(session: ShellSession): Record<string, string> {
    const env: Record<string, string> = {}
    for (const name of session.state.exportedVars ?? []) {
      const value = session.state.env.get(name)
      if (value !== undefined) env[name] = value
    }
    return env
  }

  private async raceForeground(
    session: ShellSession,
    record: ShellCommandRecord,
    foreground: ForegroundProcess | undefined,
    execPromise: Promise<ForkExecResult | Error>,
    input: { timeoutMs: number; signal?: AbortSignal; maxOutputBytes?: number },
  ): Promise<ShellCommandSnapshot> {
    const timeoutMs = normalizeTimeoutMs(input.timeoutMs)
    const operationStartedAt = Date.now()

    while (true) {
      const foregroundNow = session.foreground
      if (foregroundNow && foregroundNow !== foreground) foreground = foregroundNow

      const boundary = this.waitForBoundary(
        session,
        foreground,
        operationStartedAt,
        timeoutMs,
        input.signal,
      )

      const outcome = await Promise.race([
        execPromise.then((r) => ({ kind: 'done' as const, result: r })),
        boundary.promise,
      ])

      boundary.cancel()

      if (outcome.kind === 'done') {
        return this.collectExited(session, record, outcome.result, foreground, input)
      }
      if (outcome.kind === 'foreground_appeared') {
        foreground = outcome.foreground
        continue
      }
      if (outcome.kind === 'timeout') {
        return this.snapshotCommand(record, input)
      }
      if (outcome.kind === 'aborted') {
        const activeForeground = foreground ?? session.foreground
        if (!activeForeground) return this.collectAbortedWithoutForeground(session, record, input)
        return this.collectAborted(session, record, activeForeground, input)
      }
    }
  }

  private waitForBoundary(
    session: ShellSession,
    foreground: ForegroundProcess | undefined,
    operationStartedAt: number,
    timeoutMs: number,
    externalSignal?: AbortSignal,
  ): { promise: Promise<BoundaryOutcome>; cancel: () => void } {
    const timers: Array<() => void> = []
    const listeners: Array<() => void> = []
    const cancel = (): void => {
      for (const clear of timers) clear()
      for (const remove of listeners) remove()
    }

    if (!foreground) {
      if (session.foreground) {
        foreground = session.foreground
      } else {
        const promise = new Promise<BoundaryOutcome>((resolve) => {
          if (externalSignal?.aborted) {
            resolve({ kind: 'aborted' })
            return
          }
          const now = Date.now()
          const timeoutIn = Math.max(0, timeoutMs - (now - operationStartedAt))
          const t = setTimeout(() => resolve({ kind: 'timeout' }), timeoutIn)
          timers.push(() => clearTimeout(t))
          const onForeground = (nextForeground: ForegroundProcess): void => {
            resolve({ kind: 'foreground_appeared', foreground: nextForeground })
          }
          session.foregroundWaiters.add(onForeground)
          listeners.push(() => session.foregroundWaiters.delete(onForeground))
          if (externalSignal) {
            const onExternal = (): void => resolve({ kind: 'aborted' })
            externalSignal.addEventListener('abort', onExternal, { once: true })
            listeners.push(() => externalSignal.removeEventListener('abort', onExternal))
          }
        })
        return { promise, cancel }
      }
    }

    const fg = foreground as ForegroundProcess
    const promise = new Promise<BoundaryOutcome>((resolve) => {
      if (fg.abortController.signal.aborted || externalSignal?.aborted) {
        resolve({ kind: 'aborted' })
        return
      }

      const now = Date.now()
      const timeoutIn = Math.max(0, timeoutMs - (now - operationStartedAt))

      const t = setTimeout(() => resolve({ kind: 'timeout' }), timeoutIn)
      timers.push(() => clearTimeout(t))

      const onAbort = (): void => resolve({ kind: 'aborted' })
      fg.abortController.signal.addEventListener('abort', onAbort, { once: true })
      listeners.push(() => fg.abortController.signal.removeEventListener('abort', onAbort))
      if (externalSignal) {
        const onExternal = (): void => resolve({ kind: 'aborted' })
        externalSignal.addEventListener('abort', onExternal, { once: true })
        listeners.push(() => externalSignal.removeEventListener('abort', onExternal))
      }
    })

    return { promise, cancel }
  }

  private async hostSpawn(
    session: ShellSession,
    command: string,
    args: string[],
    opts: { cwd: string; env: Record<string, string>; stdin: string; stdinProvided?: boolean; redirections?: HostSpawnRedirection[] },
  ): Promise<ForkExecResult> {
    if (session.foreground) {
      throw new Error(`hostSpawn: session "${session.id}" already has a foreground process`)
    }
    const handle = await this.host.process.spawn({
      command,
      args,
      cwd: opts.cwd,
      env: opts.env,
      killProcessGroup: true,
    })
    const startedAt = Date.now()
    const abortController = new AbortController()
    const commandId = session.activeCommandId
    if (!commandId) throw new Error(`hostSpawn: session "${session.id}" has no active command`)
    const foreground: ForegroundProcess = {
      commandId,
      command,
      args,
      cwd: opts.cwd,
      handle,
      startedAt,
      lastOutputAt: startedAt,
      rawStdoutBuffer: '',
      rawStderrBuffer: '',
      stdoutBuffer: '',
      stderrBuffer: '',
      outputChunks: [],
      outputBytes: 0,
      lastStdoutSnapshot: 0,
      lastStderrSnapshot: 0,
      lastRawStdoutBytesSnapshot: 0,
      lastRawStderrBytesSnapshot: 0,
      lastStdoutBytesSnapshot: 0,
      lastStderrBytesSnapshot: 0,
      rawStdoutBytes: 0,
      rawStderrBytes: 0,
      totalStdoutBytes: 0,
      totalStderrBytes: 0,
      audit: [{ kind: 'system-command', name: command, args, cwd: opts.cwd, exitCode: 0 }],
      stdoutPump: Promise.resolve(),
      stderrPump: Promise.resolve(),
      exitPromise: handle.wait(),
      outputSinks: createOutputSinks(session.fs, opts.cwd, opts.redirections),
      abortController,
      outputLimitWaiters: new Set(),
      redirectedStdoutBytes: 0,
      redirectedStderrBytes: 0,
    }
    session.foreground = foreground
    notifyForegroundWaiters(session.foregroundWaiters, foreground)

    if (opts.stdin && opts.stdin.length > 0) {
      await handle.writeStdin(encodeUtf8(opts.stdin))
    }
    if (opts.stdinProvided) {
      await handle.closeStdin()
    }

    if (handle.output) {
      foreground.stdoutPump = pumpOutputStream(handle.output, (chunk) => {
        recordForegroundChunk(session, foreground, chunk.stream === 'stdout' ? 1 : 2, chunk.chunk)
      })
      foreground.stderrPump = Promise.resolve()
    } else {
      foreground.stdoutPump = pumpStream(handle.stdout, (chunk) => recordForegroundChunk(session, foreground, 1, chunk))
      foreground.stderrPump = pumpStream(handle.stderr, (chunk) => recordForegroundChunk(session, foreground, 2, chunk))
    }

    const exit = await foreground.exitPromise
    await Promise.allSettled([foreground.stdoutPump, foreground.stderrPump])

    const stdout = foreground.rawStdoutBuffer
    const exitCode = exit.exitCode ?? 127
    const stderr =
      exit.exitCode === null && foreground.rawStderrBuffer.length === 0
        ? `${command}: ${exit.signal ?? 'command not found'}\n`
        : foreground.rawStderrBuffer

    foreground.audit[0] = { kind: 'system-command', name: command, args, cwd: opts.cwd, exitCode }
    session.accumulator.audit.push(...foreground.audit)
    const record = this.commandsById.get(commandId)
    if (record) {
      record.stdout = foreground.stdoutBuffer
      record.stderr = foreground.stderrBuffer
      record.outputChunks = [...foreground.outputChunks]
      record.lastOutputAt = foreground.lastOutputAt
    }
    session.foreground = undefined

    return { stdout, stderr, exitCode }
  }

  private collectExited(
    session: ShellSession,
    record: ShellCommandRecord,
    resultOrError: ForkExecResult | Error,
    foreground: ForegroundProcess | undefined,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number } = {},
  ): ShellCommandSnapshot {
    if (record.status !== 'running') return this.snapshotCommand(record, input)
    if (resultOrError instanceof Error) {
      if (resultOrError instanceof ExitError) {
        session.exited = true
        const err = resultOrError as unknown as { stdout: string; stderr: string; exitCode: number }
        const outText = decodeBytesToUtf8(err.stdout)
        const errText = decodeBytesToUtf8(err.stderr)
        session.accumulator.stdout += outText
        session.accumulator.stderr += errText
        appendRecordOutput(record, 'stdout', outText)
        appendRecordOutput(record, 'stderr', errText)
        return this.finishExited(session, record, err.exitCode, input)
      }
      if (resultOrError instanceof ExecutionLimitError) {
        const text = `bash: execution limit exceeded: ${resultOrError.message}\n`
        session.accumulator.stderr += text
        appendRecordOutput(record, 'stderr', text)
        return this.finishExited(session, record, ExecutionLimitError.EXIT_CODE, input)
      }
      if (resultOrError instanceof ParseException || resultOrError instanceof LexerError) {
        const text = `bash: ${(resultOrError as Error).message}\n`
        session.accumulator.stderr += text
        appendRecordOutput(record, 'stderr', text)
        return this.finishExited(session, record, 2, input)
      }
      if (resultOrError.message.startsWith('Unsupported shell syntax:')) {
        session.pendingExec = undefined
        throw resultOrError
      }
      if (resultOrError instanceof ArithmeticError || resultOrError instanceof BadSubstitutionError) {
        session.pendingExec = undefined
        throw resultOrError
      }
      const text = `bash: ${(resultOrError as Error).message}\n`
      session.accumulator.stderr += text
      appendRecordOutput(record, 'stderr', text)
      return this.finishExited(session, record, 1, input)
    }

    // The interpreter carries built-in command stdout as a latin1 byte string
    // (each char = one raw byte, for binary transparency). Foreground output
    // streamed from host-spawned processes is already decoded to Unicode (see
    // recordForegroundChunk). Decode the byte-string result at this boundary —
    // the same conversion Bash.exec applies — so UTF-8 text (CJK, emoji) reads
    // back correctly instead of as mojibake. decodeBytesToUtf8 leaves already-
    // Unicode and pure-ASCII strings untouched, and preserves invalid-UTF-8
    // binary as-is.
    const stdoutText = foreground ? resultOrError.stdout.slice(foreground.lastStdoutSnapshot) : decodeBytesToUtf8(resultOrError.stdout)
    const stderrText = foreground ? resultOrError.stderr.slice(foreground.lastStderrSnapshot) : decodeBytesToUtf8(resultOrError.stderr)
    session.accumulator.stdout += stdoutText
    session.accumulator.stderr += stderrText
    if (foreground) {
      record.outputChunks = [...foreground.outputChunks]
    } else if (record.outputChunks.length === 0) {
      appendRecordOutput(record, 'stdout', stdoutText)
      appendRecordOutput(record, 'stderr', stderrText)
    }
    return this.finishExited(session, record, resultOrError.exitCode, input)
  }

  private finishExited(
    session: ShellSession,
    record: ShellCommandRecord,
    exitCode: number,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number },
  ): ShellCommandSnapshot {
    const snapshot = snapshotFromAccumulator(session, session.accumulator)
    record.stdout = snapshot.stdoutTail.length === snapshot.stdoutDelta.length ? snapshot.stdoutDelta : session.accumulator.stdout
    record.stderr = snapshot.stderrTail.length === snapshot.stderrDelta.length ? snapshot.stderrDelta : session.accumulator.stderr
    if (record.outputChunks.length === 0) {
      appendRecordOutput(record, 'stdout', record.stdout)
      appendRecordOutput(record, 'stderr', record.stderr)
    }
    ensureRecordOutputCoverage(record)
    record.lastOutputAt = Date.now()
    record.status = 'exited'
    record.exitCode = exitCode
    record.audit = [...session.accumulator.audit]
    record.commandMetadata = [...session.accumulator.commandMetadata]
    session.pendingExec = undefined
    if (session.activeCommandId === record.id) session.activeCommandId = undefined
    return this.snapshotCommand(record, input)
  }

  private async collectAborted(
    session: ShellSession,
    record: ShellCommandRecord,
    foreground: ForegroundProcess,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number } = {},
  ): Promise<ShellCommandSnapshot> {
    if (record.status !== 'running') return this.snapshotCommand(record, input)
    foreground.abortController.abort()
    foreground.handle.kill('SIGTERM').catch(() => {})
    await flushForegroundSinks(session, foreground)
    const snapshot = snapshotFromForeground(session, foreground)
    record.stdout = foreground.stdoutBuffer
    record.stderr = foreground.stderrBuffer
    record.outputChunks = [...foreground.outputChunks]
    record.lastOutputAt = Date.now()
    record.status = 'aborted'
    session.foreground = undefined
    session.pendingExec = undefined
    if (session.activeCommandId === record.id) session.activeCommandId = undefined
    void snapshot
    return this.snapshotCommand(record, input)
  }

  private collectAbortedWithoutForeground(
    session: ShellSession,
    record: ShellCommandRecord,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number } = {},
  ): ShellCommandSnapshot {
    if (record.status !== 'running') return this.snapshotCommand(record, input)
    session.abortController?.abort()
    session.pendingExec = undefined
    if (session.activeCommandId === record.id) session.activeCommandId = undefined
    const snapshot = snapshotFromAccumulator(session, session.accumulator)
    record.stdout = snapshot.stdoutDelta
    record.stderr = snapshot.stderrDelta
    if (record.outputChunks.length === 0) {
      appendRecordOutput(record, 'stdout', record.stdout)
      appendRecordOutput(record, 'stderr', record.stderr)
    }
    record.lastOutputAt = Date.now()
    record.status = 'aborted'
    return this.snapshotCommand(record, input)
  }

  private snapshotCommand(
    record: ShellCommandRecord,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number } = {},
  ): ShellCommandSnapshot {
    const session = this.shells.get(record.shellId)
    const foreground = session?.foreground
    if (record.status === 'running' && foreground?.commandId === record.id) {
      record.stdout = foreground.stdoutBuffer
      record.stderr = foreground.stderrBuffer
      record.outputChunks = [...foreground.outputChunks]
      record.lastOutputAt = foreground.lastOutputAt
    }
    const maxOutputBytes = input.maxOutputBytes ?? this.defaultOutputLimitBytes
    const stdout = streamArtifact(record, 'stdout', input.stdoutOffset, maxOutputBytes)
    const stderr = streamArtifact(record, 'stderr', input.stderrOffset, maxOutputBytes)
    const output = streamOutputArtifact(record, input.outputOffset, maxOutputBytes)
    const base = {
      shellId: record.shellId,
      commandId: record.id,
      stdout,
      stderr,
      output,
      runningMs: Date.now() - record.startedAt,
      idleMs: Date.now() - record.lastOutputAt,
    }
    this.persistCommandArtifact(record)
    if (record.status === 'exited') {
      const result: ShellCommandSnapshot = {
        ...base,
        status: 'exited',
        exitCode: record.exitCode ?? 0,
        audit: record.audit,
      }
      if (record.commandMetadata.length > 0) result.commandMetadata = record.commandMetadata
      return result
    }
    if (record.status === 'aborted') return { ...base, status: 'aborted' }
    return { ...base, status: 'running' }
  }

  private async lookupVirtualArtifact(scopeId: string, path: string): Promise<VirtualFileSystemNode | null> {
    const parts = path.split('/').filter(Boolean)
    if (parts.length === 1 && parts[0] === '@') return virtualDirectory(['commands'])
    if (parts.length === 2 && parts[0] === '@' && parts[1] === 'commands') {
      return virtualDirectory(await this.commandArtifactIds(scopeId))
    }
    if (parts.length === 3 && parts[0] === '@' && parts[1] === 'commands') {
      const artifact = await this.commandArtifact(scopeId, parts[2]!)
      if (!artifact) return null
      return virtualDirectory(['meta.json', 'stderr.txt', 'stdout.txt'])
    }
    if (parts.length !== 4 || parts[0] !== '@' || parts[1] !== 'commands') return null

    const artifact = await this.commandArtifact(scopeId, parts[2]!)
    if (!artifact) return null

    const fileName = parts[3]
    if (fileName === 'stdout.txt') return virtualFile(encodeUtf8(artifact.stdout))
    if (fileName === 'stderr.txt') return virtualFile(encodeUtf8(artifact.stderr))
    if (fileName === 'meta.json') return virtualFile(encodeUtf8(`${JSON.stringify(commandArtifactMeta(artifact), null, 2)}\n`))
    return null
  }

  private async commandArtifactIds(scopeId: string): Promise<string[]> {
    const ids = new Set<string>()
    for (const record of this.commandsById.values()) {
      if (record.commandScopeId === scopeId && !this.artifacts.isReleased(scopeId, record.id)) ids.add(record.id)
    }
    const storage = this.artifacts.storageFor(scopeId)
    const keys = await storage.list('commands').catch(() => [])
    for (const key of keys) {
      const match = /^commands\/([^/]+)\/artifact\.json$/.exec(key)
      if (match && !this.artifacts.isReleased(scopeId, match[1]!)) ids.add(match[1]!)
    }
    return [...ids]
  }

  private async commandArtifact(scopeId: string, commandId: string): Promise<PersistedShellCommandArtifact | null> {
    if (this.artifacts.isReleased(scopeId, commandId)) return null
    const record = this.commandsById.get(commandId)
    if (record?.commandScopeId === scopeId) {
      this.syncRunningRecord(record)
      return persistedArtifactFromRecord(record)
    }
    const value = await this.artifacts
      .storageFor(scopeId)
      .readJson<PersistedShellCommandArtifact>(`commands/${commandId}/artifact.json`)
      .catch(() => null)
    return isPersistedShellCommandArtifact(value) ? value : null
  }

  private persistCommandArtifact(record: ShellCommandRecord): void {
    this.artifacts.persist(record.commandScopeId, record.id, persistedArtifactFromRecord(record))
  }

  private syncRunningRecord(record: ShellCommandRecord): void {
    const session = this.shells.get(record.shellId)
    const foreground = session?.foreground
    if (record.status === 'running' && foreground?.commandId === record.id) {
      record.stdout = foreground.stdoutBuffer
      record.stderr = foreground.stderrBuffer
      record.outputChunks = [...foreground.outputChunks]
      record.lastOutputAt = foreground.lastOutputAt
    }
  }
}

function createPortableCommands(session: ShellSession): ForkCommand[] {
  return createLazyCommands(DEMI_PORTABLE_COMMANDS).map((command) => ({
    ...command,
    execute: async (args, ctx) => {
      const result = await command.execute(args, ctx)
      session.accumulator.audit.push({
        kind: 'portable-command',
        name: command.name,
        args,
        cwd: ctx.cwd,
        exitCode: result.exitCode,
      })
      return result
    },
  }))
}

function normalizeTimeoutMs(value: number): number {
  if (!Number.isFinite(value) || value < 1 || value > MAX_TIMEOUT_MS) {
    throw new Error(`timeoutMs must be between 1 and ${MAX_TIMEOUT_MS}`)
  }
  return Math.floor(value)
}

function streamArtifact(
  record: ShellCommandRecord,
  stream: 'stdout' | 'stderr',
  explicitOffset: number | undefined,
  maxOutputBytes: number,
): StreamArtifact {
  const text = stream === 'stdout' ? record.stdout : record.stderr
  const totalBytes = utf8Bytes(text)
  const offset = explicitOffset ?? (stream === 'stdout' ? record.stdoutOffset : record.stderrOffset)
  const boundedOffset = clampOffset(offset, totalBytes)
  const available = Math.max(0, totalBytes - boundedOffset)
  const byteLimit = Math.max(0, Math.floor(maxOutputBytes))
  const takeBytes = byteLimit === 0 ? available : Math.min(available, byteLimit)
  const delta = utf8Slice(text, boundedOffset, boundedOffset + takeBytes)
  const nextOffset = boundedOffset + utf8Bytes(delta)
  const truncated = nextOffset < totalBytes
  if (explicitOffset === undefined) {
    if (stream === 'stdout') record.stdoutOffset = nextOffset
    else record.stderrOffset = nextOffset
  }
  return {
    path: `/@/commands/${record.id}/${stream}.txt`,
    offset: nextOffset,
    delta,
    tail: tailString(text),
    bytes: totalBytes,
    truncated,
  }
}

function streamOutputArtifact(
  record: ShellCommandRecord,
  explicitOffset: number | undefined,
  maxOutputBytes: number,
): ShellOutputArtifact {
  const totalBytes = record.outputChunks.reduce((total, chunk) => total + chunk.bytes, 0)
  const offset = clampOffset(explicitOffset ?? record.outputOffset, totalBytes)
  const byteLimit = Math.max(0, Math.floor(maxOutputBytes))
  const available = Math.max(0, totalBytes - offset)
  let remaining = byteLimit === 0 ? available : Math.min(available, byteLimit)
  const chunks: ShellOutputChunk[] = []

  for (const chunk of record.outputChunks) {
    if (remaining <= 0) break
    const chunkStart = chunk.offset
    const chunkEnd = chunk.offset + chunk.bytes
    if (chunkEnd <= offset) continue
    const start = Math.max(0, offset - chunkStart)
    const take = Math.min(chunk.bytes - start, remaining)
    const text = utf8Slice(chunk.text, start, start + take)
    if (text.length > 0) {
      chunks.push({ stream: chunk.stream, text })
      remaining -= utf8Bytes(text)
    } else {
      remaining -= take
    }
  }

  const text = chunks.map((chunk) => chunk.text).join('')
  const nextOffset = offset + utf8Bytes(text)
  const truncated = nextOffset < totalBytes
  if (explicitOffset === undefined) record.outputOffset = nextOffset
  return {
    path: `demi://shell/${record.shellId}/commands/${record.id}/output`,
    offset: nextOffset,
    text,
    tail: tailOutputText(record.outputChunks),
    chunks,
    bytes: totalBytes,
    truncated,
  }
}

function appendRecordOutput(record: ShellCommandRecord, stream: 'stdout' | 'stderr', text: string): void {
  if (text.length === 0) return
  const offset = record.outputChunks.reduce((total, chunk) => total + chunk.bytes, 0)
  record.outputChunks.push({ stream, text, offset, bytes: utf8Bytes(text) })
}

function ensureRecordOutputCoverage(record: ShellCommandRecord): void {
  const stdoutBytes = record.outputChunks
    .filter((chunk) => chunk.stream === 'stdout')
    .reduce((total, chunk) => total + chunk.bytes, 0)
  const stderrBytes = record.outputChunks
    .filter((chunk) => chunk.stream === 'stderr')
    .reduce((total, chunk) => total + chunk.bytes, 0)
  if (stdoutBytes === utf8Bytes(record.stdout) && stderrBytes === utf8Bytes(record.stderr)) return
  record.outputChunks = []
  appendRecordOutput(record, 'stdout', record.stdout)
  appendRecordOutput(record, 'stderr', record.stderr)
}

function persistedArtifactFromRecord(record: ShellCommandRecord): PersistedShellCommandArtifact {
  return {
    status: record.status,
    shellId: record.shellId,
    commandId: record.id,
    startedAt: record.startedAt,
    lastOutputAt: record.lastOutputAt,
    exitCode: record.exitCode ?? null,
    stdout: record.stdout,
    stderr: record.stderr,
  }
}

function commandArtifactMeta(artifact: PersistedShellCommandArtifact): Record<string, unknown> {
  const stdoutPath = `/@/commands/${artifact.commandId}/stdout.txt`
  const stderrPath = `/@/commands/${artifact.commandId}/stderr.txt`
  return {
    status: artifact.status,
    shellId: artifact.shellId,
    commandId: artifact.commandId,
    startedAt: artifact.startedAt,
    lastOutputAt: artifact.lastOutputAt,
    runningMs: Date.now() - artifact.startedAt,
    idleMs: Date.now() - artifact.lastOutputAt,
    exitCode: artifact.exitCode,
    stdout: { path: stdoutPath, bytes: utf8Bytes(artifact.stdout) },
    stderr: { path: stderrPath, bytes: utf8Bytes(artifact.stderr) },
  }
}

function isPersistedShellCommandArtifact(value: unknown): value is PersistedShellCommandArtifact {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false
  const record = value as Record<string, unknown>
  return (
    (record.status === 'running' || record.status === 'exited' || record.status === 'aborted') &&
    typeof record.shellId === 'string' &&
    typeof record.commandId === 'string' &&
    typeof record.startedAt === 'number' &&
    typeof record.lastOutputAt === 'number' &&
    (typeof record.exitCode === 'number' || record.exitCode === null) &&
    typeof record.stdout === 'string' &&
    typeof record.stderr === 'string'
  )
}

function tailOutputText(chunks: readonly ShellOutputRecordChunk[]): string {
  const maxChars = 4096
  let text = ''
  for (let i = chunks.length - 1; i >= 0 && text.length < maxChars; i -= 1) {
    text = `${chunks[i]!.text}${text}`
  }
  return tailString(text)
}

function clampOffset(value: number, max: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0
  return Math.min(Math.floor(value), max)
}

function utf8Bytes(text: string): number {
  return encodeUtf8(text).byteLength
}

function utf8Slice(text: string, start: number, end: number): string {
  if (start <= 0 && end >= utf8Bytes(text)) return text
  return decodeUtf8(encodeUtf8(text).slice(start, end))
}

function tailString(value: string): string {
  return tail(value, 4096)
}

void (undefined as unknown as InterpreterContext)

import { ArithmeticError, BadSubstitutionError, ExitError, ExecutionLimitError, Interpreter, type InterpreterContext, type InterpreterState } from 'just-bash/interpreter'
import type { HostSpawnRedirection } from 'just-bash/interpreter'
import type { ScriptNode } from 'just-bash/ast/types'
import { createLazyCommands, type CommandName } from 'just-bash/commands'
import { parse } from 'just-bash/parser'
import { ParseException } from 'just-bash/parser/types'
import { LexerError } from 'just-bash/parser/lexer'
import type { Command as ForkCommand, CommandRegistry as ForkCommandRegistry, ExecResult as ForkExecResult, IFileSystem } from 'just-bash/types'
import { resolveLimits } from 'just-bash/limits'
import { CommandRegistry, type CommandSpec } from './command'
import { decodeUtf8, encodeUtf8 } from './bytes'
import { extractSimpleBackgroundCommand, formatCommandDisplay } from './background-command'
import {
  buildBashopts,
  buildShellopts,
  createOutputSinks,
  emptySnapshot,
  exitedResult,
  flushForegroundSinks,
  notifyForegroundWaiters,
  pumpStream,
  recordForegroundChunk,
  runningResult,
  snapshotFromAccumulator,
  snapshotFromForeground,
} from './environment-output'
import type { BackgroundJob, BoundaryOutcome, ForegroundProcess, ShellSession } from './environment-state'
import type { Host } from './host'
import { HostBackedFileSystem } from './host-fs'
import { AgentSessionCommandStorage } from './storage'
import { commandSpecToForkCommand } from './registered-command-adapter'

export interface BashEnvironmentOptions {
  host: Host
  commands?: CommandRegistry
  shellIdFactory?: () => string
  initialEnv?: Record<string, string>
  yieldAfterMs?: number
  timeoutMs?: number
  outputLimitBytes?: number
}

export interface ShellExecInput {
  script: string
  shellId?: string
  agentSessionId?: string
  yieldAfterMs?: number
  timeoutMs?: number
  outputLimitBytes?: number
  signal?: AbortSignal
}

export interface ShellWaitInput {
  shellId: string
  yieldAfterMs?: number
  timeoutMs?: number
  outputLimitBytes?: number
  signal?: AbortSignal
}

export interface ShellStdinInput {
  shellId: string
  stdin: string | Uint8Array
  yieldAfterMs?: number
  outputLimitBytes?: number
  signal?: AbortSignal
}

export interface ShellAbortInput {
  shellId: string
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

export type ShellToolResult =
  | {
      status: 'exited'
      shellId: string
      exitCode: number
      output: OutputSnapshot
      audit: BashAuditEvent[]
      commandMetadata?: CommandMetadataRecord[]
    }
  | {
      status: 'running'
      shellId: string
      reason: 'yield' | 'output_limit'
      output: OutputSnapshot
      runningMs: number
      idleMs: number
    }
  | { status: 'timeout'; shellId: string; output: OutputSnapshot; runningMs: number }
  | { status: 'aborted'; shellId: string; output: OutputSnapshot; runningMs: number }

const DEFAULT_YIELD_AFTER_MS = 10_000
const DEFAULT_TIMEOUT_MS = 120_000
const DEFAULT_OUTPUT_LIMIT_BYTES = 1024 * 1024
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
  private readonly initialEnv: Record<string, string>
  private readonly defaultYieldAfterMs: number
  private readonly defaultTimeoutMs: number
  private readonly defaultOutputLimitBytes: number
  private readonly shells = new Map<string, ShellSession>()
  private readonly defaultShellByAgentSessionId = new Map<string, string>()

  constructor(options: BashEnvironmentOptions) {
    this.host = options.host
    this.commands = options.commands ?? new CommandRegistry()
    this.shellIdFactory = options.shellIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.initialEnv = options.initialEnv ?? {}
    this.defaultYieldAfterMs = options.yieldAfterMs ?? DEFAULT_YIELD_AFTER_MS
    this.defaultTimeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS
    this.defaultOutputLimitBytes = options.outputLimitBytes ?? DEFAULT_OUTPUT_LIMIT_BYTES
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

  async exec(input: ShellExecInput): Promise<ShellToolResult> {
    const session = input.shellId ? this.requireShell(input.shellId) : this.availableDefaultShell(input.agentSessionId)
    if (session.exited) throw new Error(`Shell session "${session.id}" has exited`)
    if (session.pendingExec) return this.raceForeground(session, session.foreground, session.pendingExec, input)
    if (session.foreground) return runningResult(session, session.foreground, 'yield')

    return this.runScript(session, input.script, input)
  }

  async wait(input: ShellWaitInput): Promise<ShellToolResult> {
    const session = this.requireShell(input.shellId)
    if (!session.pendingExec) {
      return exitedResult(session, 0, emptySnapshot())
    }
    return this.raceForeground(session, session.foreground, session.pendingExec, input)
  }

  async input(input: ShellStdinInput): Promise<ShellToolResult> {
    const session = this.requireShell(input.shellId)
    if (!session.foreground || !session.pendingExec) {
      throw new Error(`Shell session "${session.id}" has no foreground process`)
    }
    const data = typeof input.stdin === 'string' ? encodeUtf8(input.stdin) : input.stdin
    await session.foreground.handle.writeStdin(data)
    return this.raceForeground(session, session.foreground, session.pendingExec, input)
  }

  async abort(input: ShellAbortInput): Promise<ShellToolResult> {
    const session = this.requireShell(input.shellId)
    const foreground = session.foreground
    if (!foreground) {
      return {
        status: 'aborted',
        shellId: session.id,
        output: emptySnapshot(),
        runningMs: 0,
      }
    }
    foreground.abortController.abort()
    await foreground.handle.kill('SIGTERM')
    session.state.lastExitCode = 130
    return this.collectAborted(session, foreground)
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
    const fs = new HostBackedFileSystem(this.host)
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

  private async runScript(session: ShellSession, script: string, input: ShellExecInput): Promise<ShellToolResult> {
    let ast: ScriptNode
    try {
      ast = parse(script)
    } catch (error) {
      if (error instanceof ParseException || error instanceof LexerError) {
        const message = (error as Error).message
        session.accumulator.stderr += `bash: ${message}\n`
        return exitedResult(session, 2, snapshotFromAccumulator(session, session.accumulator))
      }
      throw error
    }
    session.accumulator = { stdout: '', stderr: '', audit: [], commandMetadata: [] }
    session.startStdoutBytes = session.totalStdoutBytes
    session.startStderrBytes = session.totalStderrBytes
    session.abortController = new AbortController()

    const execPromise = session.interpreter.executeScript(ast).then(
      (result) => result,
      (error) => error as Error,
    )
    session.pendingExec = execPromise
    return this.raceForeground(session, undefined, execPromise, input)
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
    foreground: ForegroundProcess | undefined,
    execPromise: Promise<ForkExecResult | Error>,
    input: { yieldAfterMs?: number; timeoutMs?: number; outputLimitBytes?: number; signal?: AbortSignal },
  ): Promise<ShellToolResult> {
    const yieldAfterMs = input.yieldAfterMs ?? this.defaultYieldAfterMs
    const timeoutMs = input.timeoutMs ?? this.defaultTimeoutMs
    const outputLimitBytes = input.outputLimitBytes ?? this.defaultOutputLimitBytes
    const operationStartedAt = Date.now()

    while (true) {
      const foregroundNow = session.foreground
      if (foregroundNow && foregroundNow !== foreground) foreground = foregroundNow

      const boundary = this.waitForBoundary(
        session,
        foreground,
        operationStartedAt,
        yieldAfterMs,
        timeoutMs,
        outputLimitBytes,
        input.signal,
      )

      const outcome = await Promise.race([
        execPromise.then((r) => ({ kind: 'done' as const, result: r })),
        boundary.promise,
      ])

      boundary.cancel()

      if (outcome.kind === 'done') {
        return this.collectExited(session, outcome.result, foreground)
      }
      if (outcome.kind === 'foreground_appeared') {
        foreground = outcome.foreground
        continue
      }
      const activeForeground = foreground ?? session.foreground
      if (!activeForeground) {
        if (outcome.kind === 'aborted') return this.collectAbortedWithoutForeground(session)
        continue
      }
      if (outcome.kind === 'yield') {
        return runningResult(session, activeForeground, 'yield')
      }
      if (outcome.kind === 'output_limit') {
        return runningResult(session, activeForeground, 'output_limit')
      }
      if (outcome.kind === 'timeout') {
        return this.collectTimeout(session, activeForeground)
      }
      if (outcome.kind === 'aborted') {
        return this.collectAborted(session, activeForeground)
      }
    }
  }

  private waitForBoundary(
    session: ShellSession,
    foreground: ForegroundProcess | undefined,
    operationStartedAt: number,
    yieldAfterMs: number,
    timeoutMs: number,
    outputLimitBytes: number,
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
      const yieldIn = Math.max(0, yieldAfterMs - (now - operationStartedAt))
      const timeoutIn = Math.max(0, timeoutMs - (now - fg.startedAt))

      if (yieldAfterMs > 0) {
        const t = setTimeout(() => resolve({ kind: 'yield' }), yieldIn)
        timers.push(() => clearTimeout(t))
      }
      if (timeoutMs > 0) {
        const t = setTimeout(() => resolve({ kind: 'timeout' }), timeoutIn)
        timers.push(() => clearTimeout(t))
      }
      if (outputLimitBytes > 0) {
        const check = (): void => {
          const produced =
            fg.rawStdoutBytes -
            fg.lastRawStdoutBytesSnapshot +
            fg.rawStderrBytes -
            fg.lastRawStderrBytesSnapshot
          if (produced >= outputLimitBytes) {
            resolve({ kind: 'output_limit' })
          } else {
            fg.outputLimitWaiters.add(check)
          }
        }
        check()
        timers.push(() => fg.outputLimitWaiters.delete(check))
      }

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
    const foreground: ForegroundProcess = {
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

    foreground.stdoutPump = pumpStream(handle.stdout, (chunk) => recordForegroundChunk(session, foreground, 1, chunk))
    foreground.stderrPump = pumpStream(handle.stderr, (chunk) => recordForegroundChunk(session, foreground, 2, chunk))

    const exit = await foreground.exitPromise
    await Promise.allSettled([foreground.stdoutPump, foreground.stderrPump])

    const stdout = foreground.rawStdoutBuffer
    const exitCode = exit.exitCode ?? 127
    const stderr =
      exit.exitCode === null && foreground.rawStderrBuffer.length === 0
        ? `${command}: ${exit.signal ?? 'command not found'}\n`
        : foreground.rawStderrBuffer

    foreground.audit[0] = { kind: 'system-command', name: command, args, cwd: opts.cwd, exitCode }
    session.foreground = undefined

    return { stdout, stderr, exitCode }
  }

  private collectExited(session: ShellSession, resultOrError: ForkExecResult | Error, foreground: ForegroundProcess | undefined): ShellToolResult {
    if (resultOrError instanceof Error) {
      if (resultOrError instanceof ExitError) {
        session.exited = true
        const err = resultOrError as unknown as { stdout: string; stderr: string; exitCode: number }
        session.accumulator.stdout += err.stdout
        session.accumulator.stderr += err.stderr
        return exitedResult(session, err.exitCode, snapshotFromAccumulator(session, session.accumulator))
      }
      if (resultOrError instanceof ExecutionLimitError) {
        session.accumulator.stderr += `bash: execution limit exceeded: ${resultOrError.message}\n`
        return exitedResult(session, ExecutionLimitError.EXIT_CODE, snapshotFromAccumulator(session, session.accumulator))
      }
      if (resultOrError instanceof ParseException || resultOrError instanceof LexerError) {
        session.accumulator.stderr += `bash: ${(resultOrError as Error).message}\n`
        return exitedResult(session, 2, snapshotFromAccumulator(session, session.accumulator))
      }
      if (resultOrError.message.startsWith('Unsupported shell syntax:')) {
        session.pendingExec = undefined
        throw resultOrError
      }
      if (resultOrError instanceof ArithmeticError || resultOrError instanceof BadSubstitutionError) {
        session.pendingExec = undefined
        throw resultOrError
      }
      session.accumulator.stderr += `bash: ${(resultOrError as Error).message}\n`
      return exitedResult(session, 1, snapshotFromAccumulator(session, session.accumulator))
    }

    session.accumulator.stdout += foreground ? resultOrError.stdout.slice(foreground.lastStdoutSnapshot) : resultOrError.stdout
    session.accumulator.stderr += foreground ? resultOrError.stderr.slice(foreground.lastStderrSnapshot) : resultOrError.stderr
    if (foreground) {
      session.accumulator.audit.push(...foreground.audit)
    }
    return exitedResult(session, resultOrError.exitCode, snapshotFromAccumulator(session, session.accumulator))
  }

  private async collectTimeout(session: ShellSession, foreground: ForegroundProcess): Promise<ShellToolResult> {
    foreground.abortController.abort()
    foreground.handle.kill('SIGTERM').catch(() => {})
    await flushForegroundSinks(session, foreground)
    const snapshot = snapshotFromForeground(session, foreground)
    session.foreground = undefined
    session.pendingExec = undefined
    return { status: 'timeout', shellId: session.id, output: snapshot, runningMs: Date.now() - foreground.startedAt }
  }

  private async collectAborted(session: ShellSession, foreground: ForegroundProcess): Promise<ShellToolResult> {
    foreground.abortController.abort()
    foreground.handle.kill('SIGTERM').catch(() => {})
    await flushForegroundSinks(session, foreground)
    const snapshot = snapshotFromForeground(session, foreground)
    session.foreground = undefined
    session.pendingExec = undefined
    return { status: 'aborted', shellId: session.id, output: snapshot, runningMs: Date.now() - foreground.startedAt }
  }

  private collectAbortedWithoutForeground(session: ShellSession): ShellToolResult {
    session.abortController?.abort()
    session.pendingExec = undefined
    return { status: 'aborted', shellId: session.id, output: snapshotFromAccumulator(session, session.accumulator), runningMs: 0 }
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

void (undefined as unknown as InterpreterContext)

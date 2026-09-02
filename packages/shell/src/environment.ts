import { concatBytes, decodeLatin1, decodeUtf8, encodeLatin1, encodeUtf8, isAbsolutePath } from '@demicodes/utils'
import { ArithmeticError, BadSubstitutionError, ExitError, ExecutionLimitError, Interpreter, type InterpreterState } from '@demicodes/just-bash/interpreter'
import type { HostSpawnRedirection } from '@demicodes/just-bash/interpreter'
import type { ScriptNode } from '@demicodes/just-bash/ast/types'
import { createLazyCommands } from '@demicodes/just-bash/commands'
import { decodeBytesToUtf8, unsafeBytesFromLatin1 } from '@demicodes/just-bash/encoding'
import { parse } from '@demicodes/just-bash/parser'
import { ParseException } from '@demicodes/just-bash/parser/types'
import { LexerError } from '@demicodes/just-bash/parser/lexer'
import type { Command as ForkCommand, CommandRegistry as ForkCommandRegistry, ExecResult as ForkExecResult, IFileSystem } from '@demicodes/just-bash/types'
import { resolveLimits } from '@demicodes/just-bash/limits'
import { CommandRegistry, type Command } from './command'
import { DEMI_PORTABLE_COMMANDS, RESERVED_COMMAND_NAMES, shouldPreferHostSpawn } from './portable-commands'
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
} from './environment-output'
import type { BackgroundJob, BoundaryOutcome, ForegroundProcess, ShellSession } from './environment-state'
import type { Host, HostSpawnError, SpawnErrorKind } from './host'
import { HostBackedFileSystem } from './host-fs'
import { AgentSessionCommandStorage } from './storage'
import { CommandArtifactStore } from './command-artifact-store'
import { commandToForkCommand } from './registered-command-adapter'
import {
  appendRecordOutput,
  commandStatusView,
  createCommandRecord,
  ensureRecordOutputCoverage,
  finalStdoutBoundary,
  type ShellCommandRecord,
} from './command-records'
import {
  DEFAULT_BINARY_LIMIT_BYTES,
  DEFAULT_CAPTURE_LIMIT_BYTES,
  DEFAULT_OUTPUT_LIMIT_BYTES,
  DEFAULT_TIMEOUT_MS,
  MAX_TIMEOUT_MS,
  normalizeTimeoutMs,
  type BashAuditEvent,
  type BinaryStdout,
  type ShellAbortInput,
  type ShellCommandStatus,
  type ShellEnvironment,
  type ShellEnvironmentOptions,
  type ShellExecInput,
  type ShellStatusInput,
  type ShellViewInput,
  type ShellWriteInput,
} from './shell-environment'

export interface BashEnvironmentOptions extends ShellEnvironmentOptions {
  host: Host
  commands?: CommandRegistry
}

export class BashEnvironment implements ShellEnvironment {
  private readonly host: Host
  private readonly commands: CommandRegistry
  private readonly shellIdFactory: () => string
  private readonly commandIdFactory: () => string
  private readonly initialEnv: Record<string, string>
  private readonly defaultOutputLimitBytes: number
  private readonly defaultBinaryLimitBytes: number
  private readonly captureLimitBytes: number
  private readonly shells = new Map<string, ShellSession>()
  private readonly defaultShellByAgentSessionId = new Map<string, string>()
  private readonly commandsById = new Map<string, ShellCommandRecord>()
  private readonly artifacts: CommandArtifactStore

  constructor(options: BashEnvironmentOptions) {
    this.host = options.host
    this.artifacts = new CommandArtifactStore(this.host)
    this.commands = options.commands ?? new CommandRegistry(RESERVED_COMMAND_NAMES)
    this.shellIdFactory = options.shellIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.commandIdFactory = options.commandIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.initialEnv = options.initialEnv ?? {}
    this.defaultOutputLimitBytes = options.maxOutputBytes ?? DEFAULT_OUTPUT_LIMIT_BYTES
    this.defaultBinaryLimitBytes = options.maxBinaryBytes ?? DEFAULT_BINARY_LIMIT_BYTES
    this.captureLimitBytes = options.maxCaptureBytes ?? DEFAULT_CAPTURE_LIMIT_BYTES
  }

  getShell(shellId: string): ShellSession | null {
    return this.shells.get(shellId) ?? null
  }

  hasCommand(commandId: string): boolean {
    return this.commandsById.has(commandId)
  }

  registerCommand(command: Command): void {
    if (this.commands.get(command.name)) return
    this.commands.register(command)
  }

  registeredCommands(): Command[] {
    return this.commands.list()
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
      const stat = await this.host.fs.stat(input.cwd).catch(() => null)
      if (!stat?.isDirectory) throw new Error(`Shell exec cwd is not a directory: ${input.cwd}`)
    }
    const session = input.shellId
      ? this.requireShell(input.shellId)
      : input.ephemeral
        ? await this.createShell(input.agentSessionId, input.cwd)
        : await this.availableDefaultShell(input.agentSessionId)
    if (session.exited) throw new Error(`Shell session "${session.id}" has exited`)
    if (session.pendingExec || session.foreground) {
      const commandId = session.activeCommandId ?? session.foreground?.commandId ?? 'unknown'
      throw new Error(`Shell session "${session.id}" is already running command "${commandId}"`)
    }

    return this.runScript(session, input.script, { ...input, timeoutMs })
  }

  async status(input: ShellStatusInput): Promise<ShellCommandStatus> {
    const record = this.requireCommand(input.commandId)
    return this.commandStatus(record, input)
  }

  async write(input: ShellWriteInput): Promise<ShellCommandStatus> {
    const record = this.requireCommand(input.commandId)
    if (record.status !== 'running') throw new Error(`Command "${record.id}" is not running`)
    const session = this.requireShell(record.shellId)
    const foreground = this.requireForegroundCommand(session, record.id)
    const data = typeof input.stdin === 'string' ? encodeUtf8(input.stdin) : input.stdin
    if (data.byteLength === 0) throw new Error('shell_write field "stdin" must not be empty; use shell_status to poll')
    await foreground.handle.writeStdin(data)
    return this.commandStatus(record, input)
  }

  async abort(input: ShellAbortInput): Promise<ShellCommandStatus> {
    const record = this.requireCommand(input.commandId)
    if (record.status !== 'running') return this.commandStatus(record, input)
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
    await this.artifacts.release(record.commandStorageId, commandId)
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
    await session.cwdHandle.close().catch(() => {})
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

  private async defaultShell(agentSessionId: string | undefined): Promise<ShellSession> {
    if (!agentSessionId) return this.createShell(undefined)
    const existingShellId = this.defaultShellByAgentSessionId.get(agentSessionId)
    const existing = existingShellId ? this.shells.get(existingShellId) : undefined
    if (existing && !existing.exited) return existing
    const shell = await this.createShell(agentSessionId)
    this.defaultShellByAgentSessionId.set(agentSessionId, shell.id)
    return shell
  }

  private async availableDefaultShell(agentSessionId: string | undefined): Promise<ShellSession> {
    const shell = await this.defaultShell(agentSessionId)
    if (!agentSessionId || shell.exited || (!shell.pendingExec && !shell.foreground)) return shell
    return this.createShell(agentSessionId)
  }

  private async createShell(agentSessionId: string | undefined, initialCwd?: string): Promise<ShellSession> {
    const id = this.shellIdFactory()
    const commandStorageId = agentSessionId ?? id
    const cwd = initialCwd ?? this.host.defaultCwd
    const fs = new HostBackedFileSystem(this.host)
    const env = new Map<string, string>()
    for (const [key, value] of Object.entries(this.initialEnv)) env.set(key, value)
    env.set('PWD', cwd)
    if (agentSessionId) env.set('DEMI_SESSION_ID', agentSessionId)
    env.set('DEMI_SHELL_ID', id)
    if (!env.has('IFS')) env.set('IFS', ' \t\n')
    if (!env.has('PS1')) env.set('PS1', '')
    if (!env.has('PS2')) env.set('PS2', '> ')
    if (!env.has('SHLVL')) env.set('SHLVL', '1')
    const cwdHandle = await this.host.process.openCwd(cwd)
    const exportedVars = new Set<string>(['PWD', 'DEMI_SHELL_ID'])
    if (agentSessionId) exportedVars.add('DEMI_SESSION_ID')
    for (const key of env.keys()) {
      if (key !== key.toLowerCase()) exportedVars.add(key)
    }
    for (const key of Object.keys(this.initialEnv)) exportedVars.add(key)
    if (!env.has('HOSTNAME')) env.set('HOSTNAME', this.host.identity.hostname)

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
      virtualUid: this.host.identity.uid,
      virtualGid: this.host.identity.gid,
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
      agentSessionId: agentSessionId ?? null,
      commandStorageId,
      state,
      fs,
      interpreter: undefined as unknown as Interpreter,
      forkCommands,
      cwdHandle,
      accumulator: { stdout: '', stderr: '', audit: [] },
      foregroundWaiters: new Set(),
      backgroundJobs: new Map(),
      nextBackgroundJobId: 1,
      exited: false,
    }
    for (const command of createPortableCommands(session)) {
      forkCommands.set(command.name, command)
    }
    const storage = new AgentSessionCommandStorage(this.host.store, commandStorageId)
    for (const command of this.commands.list()) {
      forkCommands.set(command.name, commandToForkCommand(session, command, storage, this.host, this.captureLimitBytes))
    }
    const abortController = new AbortController()
    session.abortController = abortController
    // maxOutputSize aligns with the capture limit: it bounds strings the
    // in-process (portable command) pipeline builds, the same memory-safety
    // concern the capture limit covers for real-process output.
    const limits = resolveLimits({ maxOutputSize: this.captureLimitBytes, maxCommandCount: 1_000_000, maxLoopIterations: 1_000_000, maxCallDepth: 1000, maxGlobOperations: 1_000_000 })
    const interpreter = new Interpreter(
      {
        fs: fs as IFileSystem,
        commands: forkCommands,
        limits,
        exec: async () => ({ stdout: '', stderr: '', exitCode: 0 }),
        hostSpawn: (command, args, opts) => this.hostSpawn(session, command, args, opts),
        hostResolveCommand: (name, env) => this.hostResolveCommand(session, name, env),
        hostCwd: {
          enter: (path) => session.cwdHandle.chdir(path),
          snapshot: () => session.cwdHandle.snapshot(),
        },
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
  ): Promise<ShellCommandStatus> {
    const record = this.createCommandRecord(session, script)
    record.outputLimitBytes = input.maxOutputBytes ?? this.defaultOutputLimitBytes
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
        return this.commandStatus(record, input)
      }
      throw error
    }
    session.accumulator = { stdout: '', stderr: '', audit: [] }
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
    const record = createCommandRecord({
      id,
      shellId: session.id,
      commandStorageId: session.commandStorageId,
      artifactDir: this.artifacts.dirFor(session.commandStorageId, id),
      script,
      outputLimitBytes: this.defaultOutputLimitBytes,
    })
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
      cwd: session.cwdHandle.spawnPath(),
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
      droppedStdoutChars: 0,
      droppedStderrChars: 0,
      stdoutPump: Promise.resolve(),
      stderrPump: Promise.resolve(),
      exitPromise: handle.wait(),
    }
    // Background jobs are legitimate long-runners (dev servers, watchers), so a
    // chatty one is not killed; only the most recent output within the view
    // budget is retained — `wait` is the sole consumer and it renders a view.
    const retainLimit = this.defaultOutputLimitBytes
    job.stdoutPump = pumpStream(handle.stdout, (chunk) => {
      job.stdoutBuffer += decodeUtf8(chunk)
      if (job.stdoutBuffer.length > retainLimit) {
        job.droppedStdoutChars += job.stdoutBuffer.length - retainLimit
        job.stdoutBuffer = job.stdoutBuffer.slice(-retainLimit)
      }
    })
    job.stderrPump = pumpStream(handle.stderr, (chunk) => {
      job.stderrBuffer += decodeUtf8(chunk)
      if (job.stderrBuffer.length > retainLimit) {
        job.droppedStderrChars += job.stderrBuffer.length - retainLimit
        job.stderrBuffer = job.stderrBuffer.slice(-retainLimit)
      }
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
    const stdout = job.droppedStdoutChars > 0
      ? `[... dropped ${job.droppedStdoutChars} chars of earlier stdout over the capture limit ...]\n${job.stdoutBuffer}`
      : job.stdoutBuffer
    const stderr =
      exit.exitCode === null && job.stderrBuffer.length === 0
        ? `${job.command}: ${exit.signal ?? 'command not found'}\n`
        : job.droppedStderrChars > 0
          ? `[... dropped ${job.droppedStderrChars} chars of earlier stderr over the capture limit ...]\n${job.stderrBuffer}`
          : job.stderrBuffer
    session.accumulator.audit.push({ kind: 'system-command', name: job.command, args: job.args, cwd: job.cwd, exitCode })
    return { stdout, stderr, exitCode }
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
  ): Promise<ShellCommandStatus> {
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
        return this.commandStatus(record, input)
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
    const spawnCwd = session.cwdHandle.spawnPath()
    const handle = await this.host.process.spawn({
      command,
      args,
      cwd: spawnCwd,
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
      rawStdoutBytes: [],
      rawStderrBuffer: '',
      stdoutBuffer: '',
      stderrBuffer: '',
      outputChunks: [],
      outputBytes: 0,
      capturedBytes: 0,
      captureOverflowed: false,
      audit: [{ kind: 'system-command', name: command, args, cwd: opts.cwd, exitCode: 0 }],
      stdoutPump: Promise.resolve(),
      stderrPump: Promise.resolve(),
      exitPromise: handle.wait(),
      outputSinks: createOutputSinks(session.fs, opts.cwd, opts.redirections),
      abortController,
    }
    session.foreground = foreground
    notifyForegroundWaiters(session.foregroundWaiters, foreground)

    if (opts.stdin && opts.stdin.length > 0) {
      // The interpreter hands pipe stdin over latin1-packed (string char = raw
      // byte); write those bytes, not a UTF-8 re-encode of them.
      await handle.writeStdin(encodeLatin1(opts.stdin))
    }
    if (opts.stdinProvided) {
      await handle.closeStdin()
    }

    if (handle.output) {
      foreground.stdoutPump = pumpOutputStream(handle.output, (chunk) => {
        recordForegroundChunk(foreground, chunk.stream === 'stdout' ? 1 : 2, chunk.chunk, this.captureLimitBytes)
      })
      foreground.stderrPump = Promise.resolve()
    } else {
      foreground.stdoutPump = pumpStream(handle.stdout, (chunk) => recordForegroundChunk(foreground, 1, chunk, this.captureLimitBytes))
      foreground.stderrPump = pumpStream(handle.stderr, (chunk) => recordForegroundChunk(foreground, 2, chunk, this.captureLimitBytes))
    }

    const exit = await foreground.exitPromise
    await Promise.allSettled([foreground.stdoutPump, foreground.stderrPump])

    // Return raw output bytes (latin1-packed, stdoutKind 'bytes') so real-
    // process output pipes onward losslessly; the streamed text view on the
    // record remains the lossy render for live observation.
    const stdout = foreground.captureOverflowed ? '' : decodeLatin1(concatBytes(foreground.rawStdoutBytes))
    let exitCode = foreground.captureOverflowed ? 137 : (exit.exitCode ?? 127)
    let stderr = foreground.captureOverflowed
      ? `${command}: output exceeded the ${this.captureLimitBytes}-byte capture limit and the process was killed; ` +
        `the shell buffers whole command outputs in memory — narrow the output at the source (filters, head, tighter paths)\n`
      : foreground.rawStderrBuffer
    let spawnError = exit.spawnError
    if (!foreground.captureOverflowed && exit.spawnError) {
      exitCode = spawnErrorExitCode(exit.spawnError.kind)
      stderr = spawnErrorStderr(command, opts.cwd, opts.env, exit.spawnError)
    } else if (!foreground.captureOverflowed && exit.exitCode === null && foreground.rawStderrBuffer.length === 0) {
      stderr = `${command}: ${exit.signal ?? 'command not found'}\n`
    }

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

    return { stdout, stdoutKind: 'bytes', stderr, exitCode, ...(spawnError ? { spawnError } : {}) }
  }

  private async hostResolveCommand(
    session: ShellSession,
    name: string,
    env: Record<string, string>,
  ): Promise<{ kind: 'builtin' | 'registered' | 'function' | 'file'; value: string } | null> {
    if (name.includes('/')) {
      const resolved = isAbsolutePath(name) ? name : `${session.state.cwd.replace(/\/+$/, '')}/${name}`
      try {
        const fileStat = await this.host.fs.stat(resolved)
        if (!fileStat.isDirectory) return { kind: 'file', value: name }
      } catch {
        return null
      }
      return null
    }
    const pathEnv = env.PATH ?? ''
    for (const dir of pathEnv.split(':')) {
      if (!dir) continue
      const full = isAbsolutePath(dir)
        ? `${dir.replace(/\/+$/, '')}/${name}`
        : `${session.state.cwd.replace(/\/+$/, '')}/${dir}/${name}`
      try {
        const fileStat = await this.host.fs.stat(full)
        if (fileStat.isDirectory) continue
        return { kind: 'file', value: isAbsolutePath(dir) ? full : `${dir}/${name}` }
      } catch {
        continue
      }
    }
    return null
  }

  private collectExited(
    session: ShellSession,
    record: ShellCommandRecord,
    resultOrError: ForkExecResult | Error,
    foreground: ForegroundProcess | undefined,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number } = {},
  ): ShellCommandStatus {
    if (record.status !== 'running') return this.commandStatus(record, input)
    if (resultOrError instanceof Error) {
      if (resultOrError instanceof ExitError) {
        session.exited = true
        const err = resultOrError as unknown as { stdout: string; stderr: string; exitCode: number }
        const outText = decodeBytesToUtf8(unsafeBytesFromLatin1(err.stdout))
        const errText = decodeBytesToUtf8(unsafeBytesFromLatin1(err.stderr))
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

    // Final-stream boundary. The interpreter's script-level stdout is either a
    // latin1-packed byte string (each char = one raw byte; the pipe convention)
    // or an already-decoded Unicode string (any char > 0xFF). Valid UTF-8
    // becomes text; anything else stays raw bytes on the record (binaryStdout)
    // with a placeholder in the text channel — raw binary never enters the
    // text render.
    const raw = resultOrError.stdout
    let stdoutText: string
    let binary: BinaryStdout | undefined
    if (hasWideChar(raw)) {
      stdoutText = raw
    } else {
      const bytes = encodeLatin1(raw)
      const boundary = finalStdoutBoundary(bytes, this.defaultBinaryLimitBytes, record.artifactDir)
      stdoutText = boundary.text
      binary = boundary.binary
      // The full stream goes to disk regardless of the in-memory carry cap.
      if (binary) record.pendingBinaryArtifact = bytes
    }
    const stderrText = foreground ? resultOrError.stderr : decodeBytesToUtf8(unsafeBytesFromLatin1(resultOrError.stderr))
    session.accumulator.stdout += stdoutText
    session.accumulator.stderr += stderrText
    if (binary) record.binaryStdout = binary
    if (foreground && !binary) {
      record.outputChunks = [...foreground.outputChunks]
    } else if (binary) {
      // Drop any streamed (mojibake) view of a binary stream at exit; the
      // placeholder is the canonical text render.
      record.outputChunks = []
      appendRecordOutput(record, 'stdout', stdoutText)
      appendRecordOutput(record, 'stderr', stderrText)
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
  ): ShellCommandStatus {
    record.stdout = session.accumulator.stdout
    record.stderr = session.accumulator.stderr
    if (record.outputChunks.length === 0) {
      appendRecordOutput(record, 'stdout', record.stdout)
      appendRecordOutput(record, 'stderr', record.stderr)
    }
    ensureRecordOutputCoverage(record)
    record.lastOutputAt = Date.now()
    record.status = 'exited'
    record.exitCode = exitCode
    record.audit = [...session.accumulator.audit]
    session.pendingExec = undefined
    if (session.activeCommandId === record.id) session.activeCommandId = undefined
    return this.commandStatus(record, input)
  }

  private async collectAborted(
    session: ShellSession,
    record: ShellCommandRecord,
    foreground: ForegroundProcess,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number } = {},
  ): Promise<ShellCommandStatus> {
    if (record.status !== 'running') return this.commandStatus(record, input)
    foreground.abortController.abort()
    foreground.handle.kill('SIGTERM').catch(() => {})
    await flushForegroundSinks(session, foreground)
    record.stdout = foreground.stdoutBuffer
    record.stderr = foreground.stderrBuffer
    record.outputChunks = [...foreground.outputChunks]
    record.lastOutputAt = Date.now()
    record.status = 'aborted'
    session.foreground = undefined
    session.pendingExec = undefined
    if (session.activeCommandId === record.id) session.activeCommandId = undefined
    return this.commandStatus(record, input)
  }

  private collectAbortedWithoutForeground(
    session: ShellSession,
    record: ShellCommandRecord,
    input: { stdoutOffset?: number; stderrOffset?: number; outputOffset?: number; maxOutputBytes?: number } = {},
  ): ShellCommandStatus {
    if (record.status !== 'running') return this.commandStatus(record, input)
    session.abortController?.abort()
    session.pendingExec = undefined
    if (session.activeCommandId === record.id) session.activeCommandId = undefined
    record.stdout = session.accumulator.stdout
    record.stderr = session.accumulator.stderr
    if (record.outputChunks.length === 0) {
      appendRecordOutput(record, 'stdout', record.stdout)
      appendRecordOutput(record, 'stderr', record.stderr)
    }
    record.lastOutputAt = Date.now()
    record.status = 'aborted'
    return this.commandStatus(record, input)
  }

  private commandStatus(record: ShellCommandRecord, input: ShellViewInput = {}): ShellCommandStatus {
    const session = this.shells.get(record.shellId)
    const foreground = session?.foreground
    if (record.status === 'running' && foreground?.commandId === record.id) {
      record.stdout = foreground.stdoutBuffer
      record.stderr = foreground.stderrBuffer
      record.outputChunks = [...foreground.outputChunks]
      record.lastOutputAt = foreground.lastOutputAt
    }
    return commandStatusView(record, input, this.defaultOutputLimitBytes, this.artifacts)
  }
}

function createPortableCommands(session: ShellSession): ForkCommand[] {
  return createLazyCommands([...DEMI_PORTABLE_COMMANDS]).map((command) => ({
    ...command,
    preferHostSpawn: shouldPreferHostSpawn(command.name),
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

/** True when the string contains a char > 0xFF, i.e. already-decoded Unicode text. */
function spawnErrorExitCode(kind: SpawnErrorKind): number {
  if (kind === 'permission_denied' || kind === 'is_directory') return 126
  return 127
}

function spawnErrorStderr(
  command: string,
  cwd: string,
  env: Record<string, string>,
  spawnError: HostSpawnError,
): string {
  const { kind } = spawnError
  const suffix = spawnError.detail ? ` (${spawnError.detail})` : ''
  if (kind === 'permission_denied') return `bash: ${command}: Permission denied${suffix}\n`
  if (kind === 'is_directory') return `bash: ${command}: Is a directory${suffix}\n`
  if (kind === 'cwd_unusable') return `bash: ${cwd}: No such file or directory${suffix}\n`
  if (kind === 'executable_not_found') {
    if (command.includes('/') || !env.PATH) return `bash: ${command}: No such file or directory${suffix}\n`
    return `bash: ${command}: command not found${suffix}\n`
  }
  return `bash: ${command}: ${kind}${suffix}\n`
}

function hasWideChar(value: string): boolean {
  for (let i = 0; i < value.length; i += 1) {
    if (value.charCodeAt(i) > 0xff) return true
  }
  return false
}










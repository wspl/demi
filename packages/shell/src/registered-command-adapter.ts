import { bytesStream, concatBytes, decodeLatin1, encodeLatin1, encodeUtf8 } from '@demicodes/utils'
import type { Command as ForkCommand, CommandContext as ForkCommandContext, ExecResult as ForkExecResult } from '@demicodes/just-bash/types'
import { isCommandGroup, runRegisteredCommand, type Command, type CommandIO } from './command'
import { createOutputSinks, notifyForegroundWaiters, recordForegroundChunk } from './environment-output'
import type { ForegroundProcess, ShellSession } from './environment-state'
import type { AgentSessionCommandStorage } from './storage'
import type { Host, HostSpawnExit, HostSpawnHandle } from './host'

/**
 * Runs a registered command as a shell foreground job: it exposes the same
 * control surface as a host process (abort signal, live stdout/stderr view,
 * stdin as a post-start chunk stream) through a virtual process handle, so
 * `shell_status` / `shell_write` / `shell_abort` apply uniformly.
 */
export function commandToForkCommand(
  session: ShellSession,
  command: Command,
  storage: AgentSessionCommandStorage,
  host: Host,
  captureLimitBytes: number,
): ForkCommand {
  return {
    name: command.name,
    consumesStdin: treeConsumesStdin(command),
    execute: async (args, ctx): Promise<ForkExecResult> => {
      const argv = [command.name, ...args]
      const job = new VirtualForegroundJob(session, command.name, args, ctx.cwd, captureLimitBytes)
      job.install()
      try {
        const result = await Promise.race([
          runRegisteredCommand(command, {
            argv,
            stdin: bytesStream(decodeForkStdin(ctx.stdin)),
            env: mapToRecord(ctx.env),
            cwd: ctx.cwd,
            io: job.io,
            storage,
            host,
            signal: job.signal,
            stdinStream: job.stdinChunks(),
          }),
          job.killedResult(),
        ])
        if (job.foreground.captureOverflowed) return job.overflowResult()
        session.accumulator.audit.push({
          kind: 'registered-command',
          name: command.name,
          args,
          exitCode: result.exitCode,
        })
        return { stdout: job.stdoutLatin1(), stdoutKind: 'bytes', stderr: job.stderrText(), exitCode: result.exitCode }
      } catch (error) {
        if (job.foreground.captureOverflowed) return job.overflowResult()
        const message = error instanceof Error ? error.message : String(error)
        session.accumulator.audit.push({
          kind: 'registered-command',
          name: command.name,
          args,
          exitCode: 1,
        })
        return {
          stdout: job.stdoutLatin1(),
          stdoutKind: 'bytes',
          stderr: `${job.stderrText()}${command.name}: ${message}\n`,
          exitCode: 1,
        }
      } finally {
        job.release()
      }
    },
  }
}

/**
 * In-process stand-in for an OS process. `kill` maps to the abort signal
 * (there is no OS process to signal); a SIGKILL additionally abandons the
 * run so the shell never waits on an in-process function that ignores its
 * signal.
 */
class VirtualForegroundJob {
  readonly foreground: ForegroundProcess
  readonly io: CommandIO
  private readonly abortController = new AbortController()
  private readonly stdinQueue = new StdinChunkQueue()
  private settleExit!: (exit: HostSpawnExit) => void
  private killed!: (result: never) => void
  private readonly killedPromise: Promise<never>
  private readonly exitPromise: Promise<HostSpawnExit>
  private installed = false

  constructor(
    private readonly session: ShellSession,
    command: string,
    args: string[],
    cwd: string,
    private readonly captureLimitBytes: number,
  ) {
    this.exitPromise = new Promise<HostSpawnExit>((resolve) => {
      this.settleExit = resolve
    })
    this.killedPromise = new Promise<never>((_resolve, reject) => {
      this.killed = reject as (result: never) => void
    })
    // Nobody awaits killedPromise unless raced; keep an attached handler so an
    // abandoned run never surfaces as an unhandled rejection.
    this.killedPromise.catch(() => {})
    const handle: HostSpawnHandle = {
      stdout: emptyByteStream(),
      stderr: emptyByteStream(),
      writeStdin: async (data) => this.stdinQueue.push(data),
      closeStdin: async () => this.stdinQueue.close(),
      kill: async (signal) => {
        this.abortController.abort()
        this.stdinQueue.close()
        if (signal === 'SIGKILL') {
          this.settleExit({ exitCode: 137, signal: 'SIGKILL' })
          this.killed(new Error(`${command}: killed`) as never)
        }
      },
      wait: () => this.exitPromise,
    }
    const startedAt = Date.now()
    this.foreground = {
      commandId: session.activeCommandId ?? '',
      command,
      args,
      cwd,
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
      audit: [],
      stdoutPump: Promise.resolve(),
      stderrPump: Promise.resolve(),
      exitPromise: this.exitPromise,
      outputSinks: createOutputSinks(session.fs, cwd, undefined),
      abortController: this.abortController,
    }
    this.io = {
      stdout: (data) => {
        recordForegroundChunk(this.foreground, 1, toBytes(data), this.captureLimitBytes)
      },
      stderr: (data) => {
        recordForegroundChunk(this.foreground, 2, toBytes(data), this.captureLimitBytes)
      },
    }
  }

  get signal(): AbortSignal {
    return this.abortController.signal
  }

  stdinChunks(): AsyncIterable<Uint8Array> {
    return this.stdinQueue.stream()
  }

  killedResult(): Promise<never> {
    return this.killedPromise
  }

  /** Registers this job as the session foreground so shell control verbs route here. */
  install(): void {
    if (this.session.foreground || !this.foreground.commandId) return
    this.session.foreground = this.foreground
    this.installed = true
    notifyForegroundWaiters(this.session.foregroundWaiters, this.foreground)
  }

  release(): void {
    this.stdinQueue.close()
    this.settleExit({ exitCode: 0 })
    if (this.installed && this.session.foreground === this.foreground) {
      this.session.foreground = undefined
    }
  }

  stdoutLatin1(): string {
    return decodeLatin1(concatBytes(this.foreground.rawStdoutBytes))
  }

  stderrText(): string {
    return this.foreground.rawStderrBuffer
  }

  overflowResult(): ForkExecResult {
    return {
      stdout: '',
      stdoutKind: 'bytes',
      stderr:
        `${this.foreground.command}: output exceeded the ${this.captureLimitBytes}-byte capture limit and the command was stopped; ` +
        `the shell buffers whole command outputs in memory — narrow the output at the source (filters, head, tighter paths)\n`,
      exitCode: 137,
    }
  }
}

/** Async chunk queue: each pushed chunk is delivered once, in order; close ends the stream. */
class StdinChunkQueue {
  private readonly chunks: Uint8Array[] = []
  private waiter: (() => void) | null = null
  private isClosed = false

  push(data: Uint8Array): void {
    if (this.isClosed) return
    this.chunks.push(data)
    this.wake()
  }

  close(): void {
    if (this.isClosed) return
    this.isClosed = true
    this.wake()
  }

  async *stream(): AsyncIterable<Uint8Array> {
    while (true) {
      const chunk = this.chunks.shift()
      if (chunk) {
        yield chunk
        continue
      }
      if (this.isClosed) return
      await new Promise<void>((resolve) => {
        this.waiter = resolve
      })
    }
  }

  private wake(): void {
    const waiter = this.waiter
    this.waiter = null
    waiter?.()
  }
}

function toBytes(data: string | Uint8Array): Uint8Array {
  return typeof data === 'string' ? encodeUtf8(data) : data
}

async function* emptyByteStream(): AsyncIterable<Uint8Array> {}

function treeConsumesStdin(command: Command): boolean {
  if (isCommandGroup(command)) return command.subcommands.some(treeConsumesStdin)
  return command.stdinField !== undefined
}

function mapToRecord(map: Map<string, string>): Record<string, string> {
  const record: Record<string, string> = Object.create(null)
  for (const [key, value] of map) record[key] = value
  return record
}

/** Pipes hand stdin over as a latin1-packed byte string, or as already-Unicode text. */
function decodeForkStdin(stdin: ForkCommandContext['stdin']): Uint8Array {
  if (!stdin) return new Uint8Array(0)
  if (stdin instanceof Uint8Array) return stdin
  const latin1 = stdin as unknown as string
  for (let i = 0; i < latin1.length; i += 1) {
    if (latin1.charCodeAt(i) > 0xff) return encodeUtf8(latin1)
  }
  return encodeLatin1(latin1)
}

import {
  DEFAULT_BINARY_LIMIT_BYTES,
  DEFAULT_OUTPUT_LIMIT_BYTES,
  DEFAULT_TIMEOUT_MS,
  appendRecordOutput,
  commandStatusView,
  createCommandRecord,
  finalStdoutBoundary,
  normalizeTimeoutMs,
  settleExited,
  type ShellAbortInput,
  type ShellCommandRecord,
  type ShellCommandStatus,
  type ShellEnvironment,
  type ShellEnvironmentOptions,
  type ShellExecInput,
  type ShellStatusInput,
  type ShellWriteInput,
} from '@demicodes/shell'
import { concatBytes, decodeUtf8, delay, isAbsolutePath, toBytes } from '@demicodes/utils'
import type { RemoteHost, RemoteJob, RemoteJobExit } from './remote-host'

export interface RemoteShellEnvironmentOptions extends ShellEnvironmentOptions {
  host: RemoteHost
}

interface RemoteShell {
  id: string
  agentSessionId: string | null
  commandStorageId: string
  cwd: string
  env: Record<string, string>
  foreground?: RunningJob
  exited: boolean
}

interface RunningJob {
  record: ShellCommandRecord
  job: RemoteJob
  settled: Promise<void>
  /** Set by `abort`: the exit that follows is the kill, not the command's own end. */
  aborted?: boolean
}

/** How long `abort` waits for the runner to report the job dead after `job_kill`. */
const ABORT_GRACE_MS = 5_000

/**
 * The shell behind the `shell_*` tools for a real host (`runner.md` § Jobs
 * and the tee): every exec is one job on the runner, the record holds the
 * model's view of its output — the head while it runs, the tail at exit —
 * and the full output stays in the files the runner's tee wrote. The
 * working directory the script ended in carries into the next exec of the
 * same shell; nothing else of the shell's state does.
 */
export class RemoteShellEnvironment implements ShellEnvironment {
  private readonly host: RemoteHost
  private readonly shellIdFactory: () => string
  private readonly commandIdFactory: () => string
  private readonly initialEnv: Record<string, string>
  private readonly defaultOutputLimitBytes: number
  private readonly binaryLimitBytes: number
  private readonly shells = new Map<string, RemoteShell>()
  private readonly defaultShellByAgentSessionId = new Map<string, string>()
  private readonly commandsById = new Map<string, ShellCommandRecord>()
  private readonly runningById = new Map<string, RunningJob>()

  constructor(options: RemoteShellEnvironmentOptions) {
    this.host = options.host
    this.shellIdFactory = options.shellIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.commandIdFactory = options.commandIdFactory ?? (() => globalThis.crypto.randomUUID())
    this.initialEnv = options.initialEnv ?? {}
    this.defaultOutputLimitBytes = options.maxOutputBytes ?? DEFAULT_OUTPUT_LIMIT_BYTES
    this.binaryLimitBytes = options.maxBinaryBytes ?? DEFAULT_BINARY_LIMIT_BYTES
  }

  getShell(shellId: string): { id: string } | null {
    return this.shells.get(shellId) ?? null
  }

  hasCommand(commandId: string): boolean {
    return this.commandsById.has(commandId)
  }

  async exec(input: ShellExecInput): Promise<ShellCommandStatus> {
    const timeoutMs = normalizeTimeoutMs(input.timeoutMs ?? DEFAULT_TIMEOUT_MS)
    if (input.shellId && input.ephemeral) {
      throw new Error('ShellExecInput: "shellId" and "ephemeral" are mutually exclusive')
    }
    if (input.cwd !== undefined && !input.ephemeral) {
      throw new Error('ShellExecInput: "cwd" requires "ephemeral"; a persistent shell owns its cwd')
    }
    if (input.cwd !== undefined && !isAbsolutePath(input.cwd)) {
      throw new Error(`Shell exec cwd must be absolute: ${input.cwd}`)
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
    const running = this.runningById.get(record.id)
    if (record.status !== 'running' || !running) throw new Error(`Command "${record.id}" is not running`)
    const data = toBytes(input.stdin)
    if (data.byteLength === 0) throw new Error('shell_write field "stdin" must not be empty; use shell_status to poll')
    await running.job.writeStdin(data)
    return this.view(record)
  }

  async abort(input: ShellAbortInput): Promise<ShellCommandStatus> {
    const record = this.requireCommand(input.commandId)
    const running = this.runningById.get(record.id)
    if (record.status !== 'running' || !running) return this.view(record)
    running.aborted = true
    await running.job.kill('SIGTERM')
    await settledOrElapsed(running.settled, ABORT_GRACE_MS)
    if (record.status === 'running') {
      await running.job.kill('SIGKILL')
      await settledOrElapsed(running.settled, ABORT_GRACE_MS)
    }
    if (record.status === 'running') this.markAborted(running)
    return this.view(record)
  }

  async releaseCommand(commandId: string): Promise<boolean> {
    const record = this.commandsById.get(commandId)
    if (!record) return false
    if (record.status === 'running') await this.abort({ commandId })
    this.commandsById.delete(commandId)
    // The output files are the runner's; they stay on the target with the
    // rest of the command's history.
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

  private start(shell: RemoteShell, script: string, callerSignal: AbortSignal | undefined): RunningJob {
    const id = this.commandIdFactory()
    const job = this.host.startJob({ script, cwd: shell.cwd, env: { ...shell.env, PWD: shell.cwd } })
    const record = createCommandRecord({
      id,
      shellId: shell.id,
      commandStorageId: shell.commandStorageId,
      script,
      outputLimitBytes: this.defaultOutputLimitBytes,
    })
    this.commandsById.set(id, record)
    const running: RunningJob = { record, job, settled: Promise.resolve() }
    const head = { stdout: [] as Uint8Array[], stderr: [] as Uint8Array[] }
    const decoders = { stdout: new TextDecoder(), stderr: new TextDecoder() }
    const ingest = async () => {
      for await (const chunk of job.output) {
        head[chunk.stream].push(chunk.chunk)
        const text = decoders[chunk.stream].decode(chunk.chunk, { stream: true })
        if (chunk.stream === 'stdout') record.stdout += text
        else record.stderr += text
        appendRecordOutput(record, chunk.stream, text)
        record.lastOutputAt = Date.now()
      }
    }
    if (callerSignal) {
      const onAbort = () => void job.kill('SIGTERM')
      if (callerSignal.aborted) onAbort()
      else callerSignal.addEventListener('abort', onAbort, { once: true })
    }
    running.settled = Promise.all([ingest(), job.wait()])
      .then(([, exit]) => this.finish(shell, running, exit, head))
      .finally(() => {
        this.runningById.delete(id)
        if (shell.foreground === running) shell.foreground = undefined
      })
    shell.foreground = running
    this.runningById.set(id, running)
    return running
  }

  private async finish(shell: RemoteShell, running: RunningJob, exit: RemoteJobExit, head: { stdout: Uint8Array[]; stderr: Uint8Array[] }): Promise<void> {
    const { record } = running
    if (record.status !== 'running') return
    if (exit.cwd) shell.cwd = exit.cwd
    if (exit.spawnError) {
      const message = exit.signal && exit.spawnError.kind === 'other' ? exit.signal : `bash: ${exit.spawnError.kind}`
      settleExited(record, 127, decodeUtf8(concatBytes(head.stdout)), `${decodeUtf8(concatBytes(head.stderr))}${message}\n`, undefined)
      return
    }
    const output = exit.output
    // The runner names where its tee wrote: the output files are the target's.
    if (output) record.outputDir = output.stdoutPath.slice(0, output.stdoutPath.lastIndexOf('/'))
    const stdout = streamText(concatBytes(head.stdout), output?.stdoutBytes ?? 0, output?.stdoutTail, output?.stdoutPath)
    const stderr = streamText(concatBytes(head.stderr), output?.stderrBytes ?? 0, output?.stderrTail, output?.stderrPath)
    // A binary final stream: the whole stream is on the target, read back by
    // reference within the binary ceiling so the tools can look at it.
    let binary
    let stdoutText = stdout.text
    if (output && stdout.binary && output.stdoutBytes <= this.binaryLimitBytes) {
      const bytes = await this.host.fs.readFile(output.stdoutPath).catch(() => null)
      if (bytes) {
        const boundary = finalStdoutBoundary(bytes, this.binaryLimitBytes, output.stdoutPath)
        stdoutText = boundary.text
        binary = boundary.binary
      }
    }
    // The streamed chunks are the head the model already saw; what the exit
    // adds — a gap note and the tail — follows them, so a cursor into the
    // merged view stays valid. A binary stream is re-presented by settle.
    if (!binary) {
      appendRecordOutput(record, 'stdout', stdoutText.slice(record.stdout.length))
      appendRecordOutput(record, 'stderr', stderr.text.slice(record.stderr.length))
    }
    const exitCode = exit.exitCode ?? (exit.signal === 'SIGTERM' || exit.signal === 'SIGKILL' ? 130 : 128)
    settleExited(record, exitCode, stdoutText, stderr.text, binary)
    // The view may be a head and a tail; the model is told the stream's length.
    if (output) {
      record.stdoutBytes = output.stdoutBytes
      record.stderrBytes = output.stderrBytes
    }
    // Killed by `abort`: the streams settle as above, the status is the caller's stop.
    if (running.aborted) {
      record.status = 'aborted'
      delete record.exitCode
    }
  }

  private markAborted(running: RunningJob): void {
    const { record } = running
    if (record.status !== 'running') return
    record.lastOutputAt = Date.now()
    record.status = 'aborted'
  }

  private view(record: ShellCommandRecord): ShellCommandStatus {
    record.runningHint = this.runningById.get(record.id)?.job.runningHint
    return commandStatusView(record, this.defaultOutputLimitBytes)
  }

  private requireShell(shellId: string): RemoteShell {
    const shell = this.shells.get(shellId)
    if (!shell) throw new Error(`Unknown shell session "${shellId}"`)
    return shell
  }

  private requireCommand(commandId: string): ShellCommandRecord {
    const record = this.commandsById.get(commandId)
    if (!record) throw new Error(`Unknown command "${commandId}"`)
    return record
  }

  private defaultShell(agentSessionId: string | undefined): RemoteShell {
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

  private createShell(agentSessionId: string | undefined, initialCwd?: string): RemoteShell {
    const id = this.shellIdFactory()
    const env: Record<string, string> = { ...this.initialEnv, DEMI_SHELL_ID: id }
    if (agentSessionId) env.DEMI_SESSION_ID = agentSessionId
    const shell: RemoteShell = {
      id,
      agentSessionId: agentSessionId ?? null,
      commandStorageId: agentSessionId ?? id,
      cwd: initialCwd ?? this.host.defaultCwd,
      env,
      exited: false,
    }
    this.shells.set(id, shell)
    return shell
  }
}

/**
 * The text of one stream as the model sees it: the head that streamed, and
 * when the stream outgrew the view, a gap note and the tail the runner read
 * from the output file at exit.
 */
function streamText(head: Uint8Array, totalBytes: number, tail: Uint8Array | undefined, path: string | undefined): { text: string; binary: boolean } {
  const bytes = totalBytes <= head.byteLength || !tail ? head : head.byteLength + tail.byteLength >= totalBytes ? concatBytes([head, tail.subarray(head.byteLength + tail.byteLength - totalBytes)]) : null
  if (bytes) return { text: decodeUtf8(bytes), binary: !isUtf8(bytes) }
  const hidden = totalBytes - head.byteLength - tail!.byteLength
  const note = `\n[... ${hidden} bytes not shown; the full stream is at ${path ?? '?'} ...]\n`
  const shown = concatBytes([head, tail!])
  return { text: `${decodeUtf8(head)}${note}${decodeUtf8(tail!)}`, binary: !isUtf8(shown) }
}

function isUtf8(bytes: Uint8Array): boolean {
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(bytes)
    return true
  } catch {
    return false
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

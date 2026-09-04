import type { HostSpawnError, HostSpawnHandle } from '@demicodes/shell'
import { errorMessage, noop } from '@demicodes/utils'
import { deviceFallback } from './device-env'
import { JOB_VIEW_BYTES, type BackendToRunnerMessage, type JobOutput, type PipeRef, type RunnerToBackendMessage } from '@demicodes/runner-protocol'
import type { PipeEnds } from '../pipes'

/**
 * The runner's job table (`runner.md` § Jobs and the tee): one `bash -c`
 * process per job, its stdout and stderr teed in full to output files on
 * this machine, the model's view of them crossing the wire — the first
 * `JOB_VIEW_BYTES` of each stream while the job runs, the last
 * `JOB_VIEW_BYTES` at exit. The working directory the script ended in is
 * carried back; nothing else of the shell's state is.
 *
 * Platform-neutral: the teed spawn and the file reads are injected — the
 * tinyjs runner brings the tee primitive, tests bring a JavaScript tee
 * over a local Host.
 */
export interface JobSpawnParams {
  command: string
  args: string[]
  cwd: string
  env: Record<string, string>
  /** Where the full streams go; the handle's streams yield the view only. With `stream`, the full stdout is `stdoutStream` too. */
  tee: { stdoutPath: string; stderrPath: string; viewLimit: number; stream?: boolean }
  uid?: number
  gid?: number
}

export interface JobSpawnHandle extends Omit<HostSpawnHandle, 'output' | 'wait'> {
  /** The full stdout when the spawn asked for it: the source of the job's stdout pipe. */
  stdoutStream?: AsyncIterable<Uint8Array>
  wait(): Promise<{ exitCode: number | null; signal?: string; spawnError?: HostSpawnError; stdoutBytes: number; stderrBytes: number }>
}

export interface JobTableOptions {
  /** A teed spawn: full streams to the files, the view on the handle. */
  spawn(params: JobSpawnParams): Promise<JobSpawnHandle>
  /** Directory the output files live in, one subdirectory per job. */
  outputDir: string
  fs: {
    mkdir(path: string): Promise<void>
    /** The last `bytes` of a file, or all of it when shorter. */
    readTail(path: string, bytes: number): Promise<Uint8Array>
    readFile(path: string): Promise<Uint8Array>
    rm(path: string): Promise<void>
  }
  /** Device facts a job's env falls back to when the backend named none: `PATH`, `HOME`. */
  deviceEnv: Record<string, string>
  /** Directories every job finds first in `PATH`: the root-command symlinks. */
  pathPrefix?: string[]
  /** Entries set in every job's env regardless of what the backend named: where the runner lives (`DEMI_HOME`). */
  fixedEnv?: Record<string, string>
  /** Every job runs as this user: PID 1 spawning as the guest user. */
  runAs?: { uid: number; gid: number }
  /** The device ends of a job's pipes (`runner.md` § Pipes); absent, a job with pipes reports them failed. */
  pipes?: PipeEnds
  send(message: RunnerToBackendMessage): void
}

/** The env var naming the file a job's `EXIT` trap writes the final `pwd` to (`runner.md` § Jobs and the tee). */
export const JOB_CWD_FILE_VAR = 'DEMI_JOB_CWD_FILE'
/** The env var naming the descriptor the job prelude duplicated the job's stdin onto. */
export const JOB_STDIN_FD_VAR = 'DEMI_JOB_STDIN_FD'
/** That descriptor: fixed, high, and clear of the ones scripts and tools reach for (bash 3.2 has no `{var}<&0`). */
export const JOB_STDIN_FD = 199

interface Job {
  handle: JobSpawnHandle
  cwdFile: string
}

export class JobTable {
  private readonly jobs = new Map<string, Job>()

  constructor(private readonly options: JobTableOptions) {}

  /** Running jobs, for `pong`. */
  get count(): number {
    return this.jobs.size
  }

  async handleMessage(message: BackendToRunnerMessage): Promise<void> {
    switch (message.type) {
      case 'job_start':
        await this.start(message)
        return
      case 'job_stdin':
        await this.jobs.get(message.jobId)?.handle.writeStdin(message.bytes).catch(noop)
        return
      case 'job_stdin_end':
        await this.jobs.get(message.jobId)?.handle.closeStdin().catch(noop)
        return
      case 'job_kill':
        await this.jobs.get(message.jobId)?.handle.kill(message.signal).catch(noop)
        return
      default:
        return
    }
  }

  /** Kills every running job — the connection dropped. */
  async close(): Promise<void> {
    const jobs = [...this.jobs.values()]
    this.jobs.clear()
    await Promise.all(jobs.map((job) => job.handle.kill('SIGKILL').catch(noop)))
  }

  private async start(message: Extract<BackendToRunnerMessage, { type: 'job_start' }>): Promise<void> {
    const { jobId } = message
    const dir = `${this.options.outputDir}/${jobId}`
    const cwdFile = `${dir}/cwd`
    const stdoutPath = `${dir}/stdout.txt`
    const stderrPath = `${dir}/stderr.txt`
    let handle: JobSpawnHandle
    try {
      await this.options.fs.mkdir(dir)
      handle = await this.options.spawn({
        command: 'bash',
        args: ['-c', wrapScript(message.script)],
        cwd: message.cwd,
        env: {
          ...withPathPrefix(deviceFallback(message.env, this.options.deviceEnv), this.options.pathPrefix ?? []),
          ...this.options.fixedEnv,
          [JOB_CWD_FILE_VAR]: cwdFile,
          [JOB_STDIN_FD_VAR]: String(JOB_STDIN_FD),
        },
        tee: { stdoutPath, stderrPath, viewLimit: JOB_VIEW_BYTES, ...(message.stdout ? { stream: true } : {}) },
        ...(this.options.runAs ?? {}),
      })
    } catch (error) {
      this.options.send({ type: 'job_exit', jobId, exitCode: null, signal: errorMessage(error), spawnError: { kind: 'other' } })
      return
    }
    this.jobs.set(jobId, { handle, cwdFile })
    if (message.stdin) void this.feedStdin(handle, message.stdin)
    if (message.stdout) void this.sendStdout(handle, message.stdout)
    void this.pump(jobId, handle, { stdoutPath, stderrPath, cwdFile })
  }

  /** The job's fd 0 is a pipe: its body is fetched and written in as it arrives, then stdin closes. */
  private async feedStdin(handle: JobSpawnHandle, ref: PipeRef): Promise<void> {
    try {
      if (!this.options.pipes) throw new Error('this runner has no pipe ends')
      for await (const chunk of await this.options.pipes.get(ref.url)) await handle.writeStdin(chunk)
      this.report(ref, null)
    } catch (error) {
      this.report(ref, error)
    } finally {
      await handle.closeStdin().catch(noop)
    }
  }

  /** The job's fd 1 is a pipe: the full stdout is `PUT` as the job writes it. */
  private async sendStdout(handle: JobSpawnHandle, ref: PipeRef): Promise<void> {
    const source = handle.stdoutStream
    try {
      if (!source) throw new Error('this runner cannot stream a job\'s stdout')
      if (!this.options.pipes) {
        // Released unread, so the child never blocks on a reader that is not coming.
        await source[Symbol.asyncIterator]().return?.()
        throw new Error('this runner has no pipe ends')
      }
      await this.options.pipes.put(ref.url, source)
      this.report(ref, null)
    } catch (error) {
      this.report(ref, error)
    }
  }

  private report(ref: PipeRef, error: unknown | null): void {
    this.options.send(error === null ? { type: 'pipe_done', pipeId: ref.id, ok: true } : { type: 'pipe_done', pipeId: ref.id, ok: false, error: errorMessage(error) })
  }

  private async pump(jobId: string, handle: JobSpawnHandle, files: { stdoutPath: string; stderrPath: string; cwdFile: string }): Promise<void> {
    const forward = async (stream: AsyncIterable<Uint8Array>, name: 'stdout' | 'stderr') => {
      try {
        for await (const bytes of stream) {
          if (!this.jobs.has(jobId)) break
          this.options.send({ type: 'job_output', jobId, stream: name, bytes })
        }
      } catch {
        // A failed spawn's streams may error; the exit below still reports.
      }
    }
    const [exit] = await Promise.all([handle.wait(), forward(handle.stdout, 'stdout'), forward(handle.stderr, 'stderr')])
    if (!this.jobs.delete(jobId)) return
    const output: JobOutput = {
      stdoutPath: files.stdoutPath,
      stderrPath: files.stderrPath,
      stdoutBytes: exit.stdoutBytes,
      stderrBytes: exit.stderrBytes,
      stdoutTail: await this.options.fs.readTail(files.stdoutPath, JOB_VIEW_BYTES).catch(() => new Uint8Array(0)),
      stderrTail: await this.options.fs.readTail(files.stderrPath, JOB_VIEW_BYTES).catch(() => new Uint8Array(0)),
    }
    const cwd = await this.options.fs
      .readFile(files.cwdFile)
      .then((bytes) => new TextDecoder().decode(bytes))
      .catch(() => undefined)
    await this.options.fs.rm(files.cwdFile).catch(noop)
    this.options.send({
      type: 'job_exit',
      jobId,
      exitCode: exit.exitCode,
      ...(exit.signal !== undefined ? { signal: exit.signal } : {}),
      ...(exit.spawnError ? { spawnError: exit.spawnError } : {}),
      ...(cwd !== undefined && cwd !== '' ? { cwd } : {}),
      output,
    })
  }
}

/**
 * The script with a prelude: an `EXIT` trap so bash writes the directory it
 * ends in — after an explicit `exit` too — for the backend to carry into
 * the next job (a script bash refuses to parse never runs it, and the
 * backend keeps the directory it had); and the job's stdin duplicated onto
 * a high descriptor every child inherits, so a command-mode process can
 * tell the job's live stdin from a redirection (`tinyjs.md`, `fdNode`).
 */
export function wrapScript(script: string): string {
  return [
    `trap 'printf %s "$PWD" > "$${JOB_CWD_FILE_VAR}"' EXIT`,
    `exec ${JOB_STDIN_FD}<&0`,
    script,
  ].join('\n')
}

/** Binary resolution and the home directory are device facts (`runner.md` § Responsibilities). */
function withPathPrefix(env: Record<string, string>, prefix: string[]): Record<string, string> {
  if (prefix.length === 0) return env
  const rest = (env.PATH ?? '').split(':').filter((entry) => entry !== '' && !prefix.includes(entry))
  return { ...env, PATH: [...prefix, ...rest].join(':') }
}

import type { Host, HostFileSystem, HostProcessOutputChunk, HostSpawnHandle } from '@demicodes/shell'
import { errorCode, errorMessage } from '@demicodes/utils'
import type { BackendToRunnerMessage, FsCallMessage, FsOp, FsParams, FsResult, RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { deviceFallback } from './device-env'

/**
 * Serves a `Host`'s `fs` and `process` facets over the runner protocol —
 * the runner instantiates this with its machine layer; tests can serve any
 * Host. Platform-neutral: all IO happens through the injected Host and
 * `send`. A spawn naming no `PATH` / `HOME` gets the device's own.
 */
export class HostRpcServer {
  private readonly spawns = new Map<string, HostSpawnHandle>()

  constructor(
    private readonly host: Pick<Host, 'fs' | 'process'>,
    private readonly send: (message: RunnerToBackendMessage) => void,
    private readonly deviceEnv: Record<string, string> = {},
  ) {}

  async handleMessage(message: BackendToRunnerMessage): Promise<void> {
    if (message.type.startsWith('fs_')) {
      await this.handleFsCall(message as FsCallMessage)
      return
    }
    switch (message.type) {
      case 'spawn':
        await this.handleSpawn(message)
        return
      case 'spawn_stdin': {
        const spawn = this.spawns.get(message.spawnId)
        await spawn?.writeStdin(message.bytes).catch(() => {})
        return
      }
      case 'spawn_stdin_end': {
        const spawn = this.spawns.get(message.spawnId)
        await spawn?.closeStdin().catch(() => {})
        return
      }
      case 'spawn_kill': {
        const spawn = this.spawns.get(message.spawnId)
        await spawn?.kill(message.signal).catch(() => {})
        return
      }
      default:
        // Handshake/liveness frames belong to the connection layer, not the RPC server.
        return
    }
  }

  /** Kills every in-flight spawn — call when the connection drops. */
  async close(): Promise<void> {
    const spawns = [...this.spawns.values()]
    this.spawns.clear()
    await Promise.all(spawns.map((spawn) => spawn.kill('SIGKILL').catch(() => {})))
  }

  private async handleFsCall(message: FsCallMessage): Promise<void> {
    const { type, id, ...params } = message
    const op = type.slice('fs_'.length) as FsOp
    try {
      const result = await (fsHandlers[op] as FsHandler<FsOp>)(this.host.fs, params as FsParams<FsOp>)
      this.send({ type: 'fs_ok', id, op, result: result === undefined ? null : result } as RunnerToBackendMessage)
    } catch (error) {
      const code = errorCode(error)
      this.send({ type: 'fs_error', id, ...(code ? { code } : {}), message: errorMessage(error) })
    }
  }

  private async handleSpawn(message: Extract<BackendToRunnerMessage, { type: 'spawn' }>): Promise<void> {
    const { spawnId } = message
    let handle: HostSpawnHandle
    try {
      handle = await this.host.process.spawn({
        command: message.command,
        ...(message.args ? { args: message.args } : {}),
        ...(message.cwd !== undefined ? { cwd: message.cwd } : {}),
        ...(message.env ? { env: deviceFallback(definedEnv(message.env), this.deviceEnv) } : {}),
        ...(message.killProcessGroup !== undefined ? { killProcessGroup: message.killProcessGroup } : {}),
      })
    } catch (error) {
      this.send({
        type: 'spawn_exit',
        spawnId,
        exitCode: null,
        signal: errorMessage(error),
        spawnError: { kind: 'other' },
      })
      return
    }
    this.spawns.set(spawnId, handle)
    void this.pump(spawnId, handle)
  }

  /** Streams one ordered, stream-tagged chunk sequence, then the exit. */
  private async pump(spawnId: string, handle: HostSpawnHandle): Promise<void> {
    try {
      for await (const chunk of mergedOutput(handle)) {
        if (!this.spawns.has(spawnId)) break
        this.send({ type: 'spawn_output', spawnId, stream: chunk.stream, bytes: chunk.chunk })
      }
    } catch {
      // Output streams of a failed spawn may error; the exit below still reports.
    }
    const exit = await handle.wait()
    if (!this.spawns.delete(spawnId)) return
    this.send({
      type: 'spawn_exit',
      spawnId,
      exitCode: exit.exitCode,
      ...(exit.signal !== undefined ? { signal: exit.signal } : {}),
      ...(exit.spawnError ? { spawnError: exit.spawnError } : {}),
    })
  }
}

type FsHandler<Op extends FsOp> = (fs: HostFileSystem, params: FsParams<Op>) => Promise<FsResult<Op> | void>

/** Each `fs_<op>` request to the `HostFileSystem` method it names. */
const fsHandlers: { [Op in FsOp]: FsHandler<Op> } = {
  readFile: (fs, p) => fs.readFile(p.path, { cwd: p.cwd }),
  writeFile: (fs, p) => fs.writeFile(p.path, p.data, { cwd: p.cwd, createParents: p.createParents }),
  appendFile: (fs, p) => fs.appendFile(p.path, p.data, { cwd: p.cwd, createParents: p.createParents }),
  exists: (fs, p) => fs.exists(p.path, { cwd: p.cwd }),
  stat: (fs, p) => fs.stat(p.path, { cwd: p.cwd }),
  lstat: (fs, p) => fs.lstat(p.path, { cwd: p.cwd }),
  readdir: (fs, p) => (p.withFileTypes ? fs.readdir(p.path, { cwd: p.cwd, withFileTypes: true }) : fs.readdir(p.path, { cwd: p.cwd })),
  mkdir: (fs, p) => fs.mkdir(p.path, { cwd: p.cwd, recursive: p.recursive }),
  rm: (fs, p) => fs.rm(p.path, { cwd: p.cwd, recursive: p.recursive, force: p.force }),
  cp: (fs, p) => fs.cp(p.path, p.destination, { cwd: p.cwd, recursive: p.recursive }),
  mv: (fs, p) => fs.mv(p.path, p.destination, { cwd: p.cwd }),
  chmod: (fs, p) => fs.chmod(p.path, p.mode, { cwd: p.cwd }),
  symlink: (fs, p) => fs.symlink(p.target, p.path, { cwd: p.cwd }),
  link: (fs, p) => fs.link(p.existingPath, p.path, { cwd: p.cwd }),
  readlink: (fs, p) => fs.readlink(p.path, { cwd: p.cwd }),
  realpath: (fs, p) => fs.realpath(p.path, { cwd: p.cwd }),
  utimes: (fs, p) => fs.utimes(p.path, p.atime, p.mtime, { cwd: p.cwd }),
}

function mergedOutput(handle: HostSpawnHandle): AsyncIterable<HostProcessOutputChunk> {
  if (handle.output) return handle.output
  return mergeStreams(handle.stdout, handle.stderr)
}

/**
 * Fallback for Hosts without a native merged view: interleaving across the
 * two streams is arbitrary, but per-stream ordering is preserved.
 */
async function* mergeStreams(
  stdout: AsyncIterable<Uint8Array>,
  stderr: AsyncIterable<Uint8Array>,
): AsyncIterable<HostProcessOutputChunk> {
  const queue: HostProcessOutputChunk[] = []
  let wake: (() => void) | null = null
  let open = 2
  const drain = async (stream: AsyncIterable<Uint8Array>, name: 'stdout' | 'stderr') => {
    try {
      for await (const chunk of stream) {
        queue.push({ stream: name, chunk })
        wake?.()
        wake = null
      }
    } finally {
      open -= 1
      wake?.()
      wake = null
    }
  }
  void drain(stdout, 'stdout')
  void drain(stderr, 'stderr')
  while (true) {
    const item = queue.shift()
    if (item) {
      yield item
      continue
    }
    if (open === 0) return
    await new Promise<void>((resolve) => {
      wake = resolve
    })
  }
}

function definedEnv(env: Record<string, string | undefined>): Record<string, string> {
  const defined: Record<string, string> = {}
  for (const [key, value] of Object.entries(env)) {
    if (value !== undefined) defined[key] = value
  }
  return defined
}

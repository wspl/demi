import type { Host, HostProcessOutputChunk, HostSpawnHandle } from '@demicodes/shell'
import { errorMessage } from '@demicodes/utils'
import { isHostFsOp, type BackendToRunnerMessage, type RunnerToBackendMessage } from './messages'

/**
 * Serves a `Host`'s `fs` and `process` facets over the runner protocol —
 * the runner instantiates this with a `LocalHost`; tests can serve any Host.
 * Platform-neutral: all IO happens through the injected Host and `send`.
 */
export class HostRpcServer {
  private readonly spawns = new Map<string, HostSpawnHandle>()

  constructor(
    private readonly host: Pick<Host, 'fs' | 'process'>,
    private readonly send: (message: RunnerToBackendMessage) => void,
  ) {}

  async handleMessage(message: BackendToRunnerMessage): Promise<void> {
    switch (message.type) {
      case 'fs_call':
        await this.handleFsCall(message.id, message.op, message.args)
        return
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

  private async handleFsCall(id: string, op: unknown, args: unknown[]): Promise<void> {
    if (!isHostFsOp(op)) {
      this.send({ type: 'fs_result', id, ok: false, error: { message: `Unknown fs op: ${String(op)}` } })
      return
    }
    try {
      const method = this.host.fs[op] as (...callArgs: unknown[]) => Promise<unknown>
      const result = await method.apply(this.host.fs, args)
      this.send({ type: 'fs_result', id, ok: true, result: result === undefined ? null : result })
    } catch (error) {
      const code = (error as NodeJS.ErrnoException | null)?.code
      this.send({
        type: 'fs_result',
        id,
        ok: false,
        error: { message: errorMessage(error), ...(typeof code === 'string' ? { code } : {}) },
      })
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
        ...(message.env ? { env: message.env } : {}),
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

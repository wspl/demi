import type { HostSpawnError } from '@demicodes/shell'
import { errorCode, noop } from '@demicodes/utils'
import { spawnedHandle } from './process'
import { writeAll } from './fs'

export interface TeedSpawnParams {
  command: string
  args: string[]
  cwd: string
  env: Record<string, string>
  /** With `stream`, the full stdout is also `stdoutStream` on the handle. */
  tee: { stdoutPath: string; stderrPath: string; viewLimit: number; stream?: boolean }
  uid?: number
  gid?: number
}

export interface TeedSpawnHandle {
  /** The view: the first `viewLimit` bytes of each stream, then the end. */
  stdout: AsyncIterable<Uint8Array>
  stderr: AsyncIterable<Uint8Array>
  /** The full stdout, when asked for: a pipe end's source (`runner.md` § Pipes); dropping it early stops only this copy. */
  stdoutStream?: AsyncIterable<Uint8Array>
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(signal?: string): Promise<void>
  wait(): Promise<{ exitCode: number | null; signal?: string; spawnError?: HostSpawnError; stdoutBytes: number; stderrBytes: number }>
}

/** Log every byte, retain a bounded preview, and backpressure the optional live copy. */
export async function spawnTeed(params: TeedSpawnParams): Promise<TeedSpawnHandle> {
  // Open log files before spawning so an unwritable output directory cannot orphan a job.
  const stdoutFile = await tjs.open(params.tee.stdoutPath, 'w', 0o600)
  let stderrFile: tjs.FileHandle
  try { stderrFile = await tjs.open(params.tee.stderrPath, 'w', 0o600) }
  catch (error) { await stdoutFile.close(); throw error }
  let child: tjs.Process
  try {
    child = tjs.spawn([params.command, ...params.args], {
      cwd: params.cwd, env: params.env, detached: true,
      stdin: 'pipe', stdout: 'pipe', stderr: 'pipe',
      ...(params.uid !== undefined ? { uid: params.uid } : {}),
      ...(params.gid !== undefined ? { gid: params.gid } : {}),
    })
  } catch (error) {
    await Promise.all([stdoutFile.close(), stderrFile.close()])
    const kind: HostSpawnError['kind'] = errorCode(error) === 'ENOENT' ? 'executable_not_found' : 'other'
    const empty = async function* (): AsyncIterable<Uint8Array> {}
    return {
      stdout: empty(), stderr: empty(),
      writeStdin: async () => {}, closeStdin: async () => {}, kill: async () => {},
      wait: async () => ({ exitCode: null, spawnError: { kind }, stdoutBytes: 0, stderrBytes: 0 }),
    }
  }
  const handle = spawnedHandle(child, true)
  const stdout = logStream(handle.stdout, stdoutFile, params.tee.viewLimit, params.tee.stream === true)
  const stderr = logStream(handle.stderr, stderrFile, params.tee.viewLimit, false)
  let finished = false
  const wait = Promise.all([handle.wait(), stdout.done, stderr.done]).then(([exit, stdoutBytes, stderrBytes]) => { finished = true; return { ...exit, stdoutBytes, stderrBytes } })
  wait.catch(noop)
  return { ...handle, stdout: stdout.preview, stderr: stderr.preview,
    ...(stdout.live ? { stdoutStream: stdout.live } : {}),
    kill: async (signal = 'SIGTERM') => {
      // A surviving descendant keeps its process group reserved after the leader exits.
      // Continue allowing escalation until the whole output lifetime has ended.
      if (finished) return
      try { tjs.kill(-child.pid, signal as tjs.Signal) }
      catch (error) { if (errorCode(error) !== 'ESRCH') throw error }
    },
    wait: () => wait }
}

function logStream(source: AsyncIterable<Uint8Array>, file: tjs.FileHandle, limit: number, stream: boolean) {
  let previewController: ReadableStreamDefaultController<Uint8Array> | undefined
  const preview = new ReadableStream<Uint8Array>({
    start(controller) { previewController = controller },
    cancel() { previewController = undefined },
  })
  const live = stream ? new TransformStream<Uint8Array, Uint8Array>() : undefined
  let writer = live?.writable.getWriter()
  // Cancellation discards only the live copy. Logging continues until process EOF.
  writer?.closed.catch(() => { writer = undefined })
  const done = (async () => {
    let total = 0
    try {
      for await (const chunk of source) {
        await writeAll(file, chunk)
        const remaining = limit - total
        if (previewController && remaining > 0) previewController.enqueue(chunk.slice(0, remaining))
        total += chunk.byteLength
        if (previewController && total >= limit) { previewController.close(); previewController = undefined }
        if (writer) await writer.write(chunk).catch(() => { writer = undefined })
      }
      previewController?.close()
      await writer?.close().catch(noop)
      return total
    } catch (error) {
      previewController?.error(error)
      await writer?.abort(error).catch(noop)
      throw error
    } finally { await file.close() }
  })()
  done.catch(noop)
  return { preview, live: live?.readable, done }
}

/** Read at most the requested tail, without loading the full log. */
export async function readTail(path: string, bytes: number): Promise<Uint8Array> {
  const file = await tjs.open(path, 'r')
  try {
    const size = (await file.stat()).size
    const result = new Uint8Array(Math.min(size, bytes))
    let read = 0
    while (read < result.length) {
      const count = await file.read(result.subarray(read), size - result.length + read)
      if (count === null) break
      read += count
    }
    return result.subarray(0, read)
  } finally { await file.close() }
}

// The runner's jobs over the tee primitive, and the file reads the job table
// needs (`runner.md` § Jobs and the tee).
import * as fs from 'tinyjs:fs'
import * as proc from 'tinyjs:process'
import type { HostSpawnError } from '@demicodes/shell'
import { errorCode } from '@demicodes/utils'
import { readHandle } from './stdio'

export interface TeedSpawnParams {
  command: string
  args: string[]
  cwd: string
  env: Record<string, string>
  tee: { stdoutPath: string; stderrPath: string; viewLimit: number }
}

export interface TeedSpawnHandle {
  /** The view: the first `viewLimit` bytes of each stream, then the end. */
  stdout: AsyncIterable<Uint8Array>
  stderr: AsyncIterable<Uint8Array>
  writeStdin(data: Uint8Array): Promise<void>
  closeStdin(): Promise<void>
  kill(signal?: string): Promise<void>
  wait(): Promise<{ exitCode: number | null; signal?: string; spawnError?: HostSpawnError; stdoutBytes: number; stderrBytes: number }>
}

/**
 * Spawns a process in its own process group with both streams teed to
 * files by tinyjs; the handle's streams carry the view. A spawn failure is
 * reported through `wait` as a `HostSpawnError`, like a Host spawn.
 */
export async function spawnTeed(params: TeedSpawnParams): Promise<TeedSpawnHandle> {
  let child: proc.Child
  try {
    child = await proc.spawn({
      command: params.command,
      args: params.args,
      cwd: params.cwd,
      env: params.env,
      stdin: 'pipe',
      processGroup: true,
      tee: params.tee,
    })
  } catch (error) {
    const kind: HostSpawnError['kind'] = errorCode(error) === 'ENOENT' ? 'executable_not_found' : 'other'
    const empty = async function* (): AsyncIterable<Uint8Array> {}
    return {
      stdout: empty(),
      stderr: empty(),
      writeStdin: async () => {},
      closeStdin: async () => {},
      kill: async () => {},
      wait: async () => ({ exitCode: null, spawnError: { kind }, stdoutBytes: 0, stderrBytes: 0 }),
    }
  }
  let stdinOpen = child.stdin !== null
  let exited = false
  const closeStdin = (): void => {
    if (child.stdin === null || !stdinOpen) return
    stdinOpen = false
    fs.close(child.stdin)
  }
  const exit = proc.wait(child.pid).then((result) => {
    exited = true
    closeStdin()
    return {
      exitCode: result.code,
      ...(result.signal !== undefined ? { signal: result.signal } : {}),
      stdoutBytes: result.stdoutBytes ?? 0,
      stderrBytes: result.stderrBytes ?? 0,
    }
  })
  exit.catch(() => {})
  return {
    stdout: readHandle(child.stdout, true),
    stderr: readHandle(child.stderr, true),
    writeStdin: async (data) => {
      if (child.stdin === null || !stdinOpen) return
      await fs.write(child.stdin, data)
    },
    closeStdin: async () => closeStdin(),
    kill: async (signal = 'SIGTERM') => {
      if (exited) return
      try {
        proc.kill(child.pid, signal, { group: true })
      } catch (error) {
        if (errorCode(error) !== 'ESRCH') throw error
      }
    },
    wait: () => exit,
  }
}

/** The last `bytes` of a file, or the whole file when it is shorter. */
export async function readTail(path: string, bytes: number): Promise<Uint8Array> {
  const size = (await fs.stat(path)).size
  const length = Math.min(size, bytes)
  if (length === 0) return new Uint8Array(0)
  const fd = await fs.open(path, 'r')
  try {
    const parts: Uint8Array[] = []
    let offset = size - length
    let remaining = length
    while (remaining > 0) {
      const chunk = await fs.read(fd, remaining, offset)
      if (chunk === null) break
      parts.push(chunk)
      offset += chunk.byteLength
      remaining -= chunk.byteLength
    }
    if (parts.length === 1) return parts[0]!
    const out = new Uint8Array(length - remaining)
    let at = 0
    for (const part of parts) {
      out.set(part, at)
      at += part.byteLength
    }
    return out
  } finally {
    fs.close(fd)
  }
}

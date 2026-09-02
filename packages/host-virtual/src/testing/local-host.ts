// The Host over this Node process's machine: `nodeFileSystem` plus child
// processes, a directory-fd cwd and the real identity. A test fixture: the
// Host tests run a shell against when they need a real directory and real
// programs. The product's machines are reached through the runner.
import { spawn } from 'node:child_process'
import { mkdtempSync } from 'node:fs'
import { hostname, tmpdir, userInfo } from 'node:os'
import { join, resolve } from 'node:path'
import { stat } from 'node:fs/promises'
import type { Readable } from 'node:stream'
import { fileHostStore } from '@demicodes/shell'
import { nodeFileSystem } from '../node'
import type {
  Host,
  HostCwd,
  HostFileSystem,
  HostIdentity,
  HostProcess,
  HostProcessOutputChunk,
  HostSpawnExit,
  HostSpawnHandle,
  HostSpawnParams,
  HostStore,
  SpawnErrorKind,
} from '@demicodes/shell'
import { LocalHostCwd } from './local-cwd'

export interface LocalHostOptions {
  storeRoot?: string
  /** Bring-your-own persistence, e.g. to wrap or gate writes; defaults to a fileHostStore rooted at storeRoot. */
  store?: HostStore
}

export class LocalHost implements Host {
  readonly defaultCwd: string
  readonly fs: HostFileSystem
  readonly process: HostProcess
  readonly store: HostStore
  readonly identity: HostIdentity

  constructor(defaultCwd: string, options: LocalHostOptions = {}) {
    this.defaultCwd = resolve(defaultCwd)
    const storeRoot = options.storeRoot ?? defaultStoreRoot(this.defaultCwd)
    this.fs = nodeFileSystem(this.defaultCwd)
    this.process = new LocalHostProcess(this.defaultCwd)
    this.store = options.store ?? fileHostStore(this.fs, resolve(storeRoot))
    const info = userInfo()
    this.identity = { uid: info.uid, gid: info.gid, hostname: hostname() }
  }
}

class LocalHostProcess implements HostProcess {
  constructor(private readonly defaultCwd: string) {}

  async openCwd(path: string): Promise<HostCwd> {
    return LocalHostCwd.open(path)
  }

  async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    const cwd = params.cwd ?? this.defaultCwd
    const child = spawn(params.command, params.args ?? [], {
      cwd,
      ...(params.env ? { env: definedEnv(params.env) } : {}),
      detached: params.killProcessGroup === true,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    let settled = false
    const waitPromise = new Promise<HostSpawnExit>((resolveWait) => {
      child.once('error', (error) => {
        if (settled) return
        settled = true
        void classifySpawnFailure(error, cwd).then((kind) => {
          resolveWait({
            exitCode: null,
            signal: error.message,
            spawnError: { kind },
          })
        })
      })
      child.once('close', (exitCode, signal) => {
        setImmediate(() => {
          if (settled) return
          settled = true
          resolveWait({ exitCode, signal: signal ?? undefined })
        })
      })
    })

    return {
      stdout: streamBytes(child.stdout),
      stderr: streamBytes(child.stderr),
      output: streamMergedOutput(child.stdout, child.stderr),
      writeStdin: async (data) => {
        if (!child.stdin || child.stdin.destroyed) return
        await new Promise<void>((resolve, reject) => {
          child.stdin.write(data, (error) => {
            if (error) reject(error)
            else resolve()
          })
        })
      },
      closeStdin: async () => {
        if (!child.stdin || child.stdin.destroyed) return
        child.stdin.end()
      },
      kill: async (signal = 'SIGTERM') => {
        if (!child.pid) return
        if (params.killProcessGroup === true) {
          try {
            process.kill(-child.pid, signal as NodeJS.Signals)
            return
          } catch {
            // Fall through to the direct child when process-group signaling is unavailable.
          }
        }
        if (!child.killed) child.kill(signal as NodeJS.Signals)
      },
      wait: () => waitPromise,
    }
  }
}

async function* streamMergedOutput(
  stdout: Readable | null,
  stderr: Readable | null,
): AsyncIterable<HostProcessOutputChunk> {
  const queue: Array<HostProcessOutputChunk | { done: true }> = []
  let wake: (() => void) | null = null
  let open = 0

  const push = (item: HostProcessOutputChunk | { done: true }) => {
    queue.push(item)
    wake?.()
    wake = null
  }
  const attach = (stream: Readable | null, name: 'stdout' | 'stderr') => {
    if (!stream) return []
    open += 1
    let ended = false
    const onData = (chunk: Buffer) => push({ stream: name, chunk })
    const onEnd = () => {
      if (ended) return
      ended = true
      open -= 1
      if (open === 0) push({ done: true })
    }
    stream.on('data', onData)
    stream.once('end', onEnd)
    stream.once('close', onEnd)
    return [
      () => stream.off('data', onData),
      () => stream.off('end', onEnd),
      () => stream.off('close', onEnd),
    ]
  }

  const cleanup = [
    ...attach(stdout, 'stdout'),
    ...attach(stderr, 'stderr'),
  ]
  if (open === 0) push({ done: true })

  try {
    while (true) {
      if (queue.length === 0) {
        await new Promise<void>((resolve) => {
          wake = resolve
        })
      }
      const item = queue.shift()
      if (!item) continue
      if ('done' in item) break
      yield item
    }
  } finally {
    for (const remove of cleanup) remove()
  }
}

/**
 * A store is throwaway but stable within the process: one temp directory
 * per working directory, so a Host re-created for the same directory (a
 * reopened session) finds its state, and a fresh process starts clean.
 */
const storeRoots = new Map<string, string>()
function defaultStoreRoot(defaultCwd: string): string {
  let root = storeRoots.get(defaultCwd)
  if (!root) {
    root = mkdtempSync(join(tmpdir(), 'demi-local-host-'))
    storeRoots.set(defaultCwd, root)
  }
  return root
}

function definedEnv(env: Record<string, string | undefined>): Record<string, string> {
  const defined: Record<string, string> = {}
  for (const [key, value] of Object.entries(env)) {
    if (value !== undefined) defined[key] = value
  }
  return defined
}

async function classifySpawnFailure(error: Error, cwd: string): Promise<SpawnErrorKind> {
  try {
    const cwdStat = await stat(cwd)
    if (!cwdStat.isDirectory()) return 'cwd_unusable'
  } catch {
    return 'cwd_unusable'
  }
  const code = 'code' in error ? String((error as { code: unknown }).code) : ''
  if (code === 'ENOENT') return 'executable_not_found'
  if (code === 'EACCES' || code === 'EPERM') return 'permission_denied'
  if (code === 'EISDIR') return 'is_directory'
  return 'other'
}

async function* streamBytes(stream: Readable | null): AsyncIterable<Uint8Array> {
  if (!stream) return
  try {
    for await (const chunk of stream) {
      if (chunk instanceof Uint8Array) {
        yield chunk
      } else {
        yield Buffer.from(String(chunk))
      }
    }
  } catch (error) {
    // A child that failed to spawn closes its stdio pipes without ending them;
    // the process never produced output, so the stream simply ends.
    if ((error as NodeJS.ErrnoException | null)?.code === 'ERR_STREAM_PREMATURE_CLOSE') return
    throw error
  }
}

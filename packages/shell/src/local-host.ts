import { spawn } from 'node:child_process'
import type { Readable } from 'node:stream'
import type { Host, HostSpawnHandle, HostSpawnParams } from './host'

export class LocalHost implements Host {
  readonly root: string

  constructor(root: string) {
    this.root = root
  }

  async spawn(params: HostSpawnParams): Promise<HostSpawnHandle> {
    const child = spawn(params.command, params.args ?? [], {
      cwd: params.cwd ?? this.root,
      env: { ...process.env, ...params.env },
      detached: params.killProcessGroup === true,
      stdio: ['pipe', 'pipe', 'pipe'],
    })

    let settled = false
    const waitPromise = new Promise<{ exitCode: number | null; signal?: string }>((resolve) => {
      child.once('error', (error) => {
        if (settled) return
        settled = true
        resolve({ exitCode: null, signal: error.message })
      })
      child.once('close', (exitCode, signal) => {
        if (settled) return
        settled = true
        resolve({ exitCode, signal: signal ?? undefined })
      })
    })

    return {
      stdout: streamBytes(child.stdout),
      stderr: streamBytes(child.stderr),
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
            process.kill(-child.pid, signal)
            return
          } catch {
            // Fall through to killing the direct child when the platform or process state
            // does not allow signaling the process group.
          }
        }
        if (!child.killed) child.kill(signal)
      },
      wait: () => waitPromise,
    }
  }
}

async function* streamBytes(stream: Readable | null): AsyncIterable<Uint8Array> {
  if (!stream) return
  for await (const chunk of stream) {
    if (chunk instanceof Uint8Array) {
      yield chunk
    } else {
      yield Buffer.from(String(chunk))
    }
  }
}

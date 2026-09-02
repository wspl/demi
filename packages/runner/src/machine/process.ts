import * as fs from 'tinyjs:fs'
import * as proc from 'tinyjs:process'
import { env as processEnv } from 'tinyjs:runtime'
import type { HostProcess, HostSpawnExit, HostSpawnHandle, HostSpawnParams, SpawnErrorKind } from '@demicodes/shell'
import { emptyByteStream, errorCode, noop } from '@demicodes/utils'
import { openRunnerCwd } from './cwd'
import { readHandle } from './stdio'

/**
 * The `HostProcess` facet over `tinyjs:process`. A child gets exactly the
 * env passed (the process's own when none is), a stdin pipe, and its own
 * process group when the caller wants to kill the group.
 */
export function createRunnerProcess(defaultCwd: string): HostProcess {
  return {
    openCwd: openRunnerCwd,
    spawn: async (params) => {
      const cwd = params.cwd ?? defaultCwd
      let child: proc.Child
      try {
        child = await proc.spawn({
          command: params.command,
          args: params.args ?? [],
          cwd,
          env: params.env ? definedEnv(params.env) : { ...processEnv },
          stdin: 'pipe',
          processGroup: params.killProcessGroup === true,
        })
      } catch (error) {
        return failedSpawn(await classifySpawnFailure(error, cwd))
      }
      return spawnedHandle(child, params.killProcessGroup === true)
    },
  }
}

function spawnedHandle(child: proc.Child, group: boolean): HostSpawnHandle {
  let stdinOpen = child.stdin !== null
  let exited = false
  const closeStdin = (): void => {
    if (child.stdin === null || !stdinOpen) return
    stdinOpen = false
    fs.close(child.stdin)
  }
  const exit: Promise<HostSpawnExit> = proc.wait(child.pid).then((result) => {
    exited = true
    // Nothing reads the pipe any more; a stdin handle nobody closed would leak.
    closeStdin()
    return { exitCode: result.code, ...(result.signal !== undefined ? { signal: result.signal } : {}) }
  })
  // A rejection is delivered to whoever calls wait(); nobody may.
  exit.catch(noop)
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
        proc.kill(child.pid, signal, { group })
      } catch (error) {
        // Reaped between the check and the call.
        if (errorCode(error) !== 'ESRCH') throw error
      }
    },
    wait: () => exit,
  }
}

function failedSpawn(kind: SpawnErrorKind): HostSpawnHandle {
  return {
    stdout: emptyByteStream(),
    stderr: emptyByteStream(),
    writeStdin: async () => {},
    closeStdin: async () => {},
    kill: async () => {},
    wait: async () => ({ exitCode: null, spawnError: { kind } }),
  }
}

/** A cwd that is gone explains the failure before the binary does. */
async function classifySpawnFailure(error: unknown, cwd: string): Promise<SpawnErrorKind> {
  try {
    if ((await fs.stat(cwd)).kind !== 'dir') return 'cwd_unusable'
  } catch {
    return 'cwd_unusable'
  }
  switch (errorCode(error)) {
    case 'ENOENT':
      return 'executable_not_found'
    case 'EACCES':
    case 'EPERM':
      return 'permission_denied'
    case 'EISDIR':
      return 'is_directory'
    default:
      return 'other'
  }
}

function definedEnv(env: Record<string, string | undefined>): Record<string, string> {
  const defined: Record<string, string> = {}
  for (const [key, value] of Object.entries(env)) {
    if (value !== undefined) defined[key] = value
  }
  return defined
}

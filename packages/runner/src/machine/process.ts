import type { HostProcess, HostSpawnExit, HostSpawnHandle, SpawnErrorKind } from '@demicodes/shell'
import { emptyByteStream, errorCode, noop } from '@demicodes/utils'
import { openRunnerCwd } from './cwd'
/**
 * The `HostProcess` facet over txiki.js. A child gets exactly the
 * env passed (the process's own when none is), a stdin pipe, and its own
 * process group when the caller wants to kill the group.
 */
export function createRunnerProcess(defaultCwd: string): HostProcess {
  return {
    openCwd: openRunnerCwd,
    spawn: async (params) => {
      const cwd = params.cwd ?? defaultCwd
      let child: tjs.Process
      try {
        child = tjs.spawn([params.command, ...(params.args ?? [])], {
          cwd,
          env: params.env ? definedEnv(params.env) : { ...tjs.env },
          stdin: 'pipe',
          stdout: 'pipe',
          stderr: 'pipe',
          detached: params.killProcessGroup === true,
        })
      } catch (error) {
        return failedSpawn(await classifySpawnFailure(error, cwd))
      }
      return spawnedHandle(child, params.killProcessGroup === true)
    },
  }
}

export function spawnedHandle(child: tjs.Process, group: boolean): HostSpawnHandle {
  const writer = child.stdin?.getWriter()
  let stdinOpen = !!writer
  let exited = false
  const closeStdin = async (): Promise<void> => {
    if (!stdinOpen || !writer) return
    stdinOpen = false
    // The child may close its input before the caller finishes sending.
    await writer.close().catch(noop)
  }
  const exit: Promise<HostSpawnExit> = child.wait().then((result) => {
    exited = true
    stdinOpen = false
    // Release pending writes after process exit; the pipe may already be closed.
    void writer?.abort().catch(noop)
    if (result.term_signal) return { exitCode: null, signal: result.term_signal }
    return { exitCode: result.exit_status }
  })
  exit.catch(noop)
  return {
    stdout: child.stdout ?? emptyByteStream(),
    stderr: child.stderr ?? emptyByteStream(),
    writeStdin: async (data) => {
      if (stdinOpen && writer) await writer.write(data)
    },
    closeStdin,
    kill: async (signal = 'SIGTERM') => {
      if (exited) return
      try {
        tjs.kill(group ? -child.pid : child.pid, signal as tjs.Signal)
      } catch (error) {
        // The process can exit between checking its status and sending the signal.
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
    if (!(await tjs.stat(cwd)).isDirectory) return 'cwd_unusable'
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

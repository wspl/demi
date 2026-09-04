// Command mode (`commands.md` § Root commands on a target): a root command's
// process. The loader runs `runtime` modules here against the machine's
// Host from the runner's manifest cache, and forwards `rpc` leaves to the
// runner over the relay.
import { createRunnerHost, cwd, env, fdNode, identity, onSignal, stderrWriter, stdinStream, stdoutWriter } from './machine'
import { createLoader, directorySource, inMemorySource, parseManifest, type ManifestSource } from '@demicodes/command-loader'
import { JOB_ID_VAR, JOB_STDIN_FD_VAR } from './serve/jobs'
import { errorMessage } from '@demicodes/utils'
import { fetchManifest, relayRpc } from './relay/client'

/** The runner's state directory: `DEMI_HOME`, else `~/.demi`. */
export function stateDir(): string {
  return env.DEMI_HOME ?? `${identity.homeDir}/.demi`
}

export async function runCommandMode(root: string, args: readonly string[]): Promise<number> {
  const host = createRunnerHost()
  const dir = stateDir()
  const socketPath = `${dir}/runner.sock`
  let source: ManifestSource
  try {
    source = await manifestSource(host.fs, env.DEMI_COMMANDS_DIR ?? `${dir}/commands/current`, socketPath)
  } catch (error) {
    await stderrWriter()(`${root}: no command manifest: ${errorMessage(error)}\n`)
    return 127
  }
  const ids = { agentSessionId: env.DEMI_SESSION_ID ?? '', shellId: env.DEMI_SHELL_ID ?? '', ...(env[JOB_ID_VAR] ? { jobId: env[JOB_ID_VAR] } : {}) }
  const loader = await createLoader({ source, host, ...(ids.agentSessionId ? { rpc: relayRpc(socketPath, ids) } : {}) })
  const controller = new AbortController()
  onSignal('SIGINT', () => controller.abort())
  onSignal('SIGTERM', () => controller.abort())
  // The job's own stdin is live — shell_write feeds it and it never ends
  // on its own — so it is the post-start stream and there is no pipe; a
  // redirection is the pipe.
  const live = isJobStdin()
  return loader.dispatch(root, args, {
    ...(live ? { stdinStream: stdinStream() } : { stdin: stdinStream() }),
    stdout: stdoutWriter(),
    stderr: stderrWriter(),
    cwd: cwd(),
    env: { ...env },
    signal: controller.signal,
  })
}

/** The cache directory when it holds a manifest; a miss asks the runner over the relay. */
async function manifestSource(fs: Parameters<typeof directorySource>[1], dir: string, socketPath: string): Promise<ManifestSource> {
  if (await fs.exists(`${dir}/manifest.json`)) return directorySource(dir, fs)
  const fetched = await fetchManifest(socketPath)
  if (fetched === null) throw new Error('the runner has no manifest yet')
  return inMemorySource(parseManifest(fetched))
}

/** Whether fd 0 is the job's stdin the prelude duplicated (`runner-protocol`, `wrapScript`). */
function isJobStdin(): boolean {
  const duplicated = Number(env[JOB_STDIN_FD_VAR])
  if (!Number.isInteger(duplicated) || duplicated < 3) return false
  const own = fdNode(0)
  return own !== null && own === fdNode(duplicated)
}

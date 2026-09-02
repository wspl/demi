// Command mode (`docs/demi-next/commands.md` § Root commands on a target): a
// root command's process. The loader runs `runtime` modules here against the
// machine's Host, from the manifest directory the runner maintains.
import { createRunnerHost, cwd, env, identity, onSignal, stderrWriter, stdinStream, stdoutWriter } from '@demicodes/host-runner'
import { createLoader, directorySource } from '@demicodes/command-loader'

/** The manifest directory: `DEMI_COMMANDS_DIR`, else the runner's current cache. */
export function commandsDir(): string {
  return env.DEMI_COMMANDS_DIR ?? `${identity.homeDir}/.demi/commands/current`
}

export async function runCommandMode(root: string, args: readonly string[]): Promise<number> {
  const host = createRunnerHost()
  const loader = await createLoader({ source: directorySource(commandsDir(), host.fs), host })
  const controller = new AbortController()
  onSignal('SIGINT', () => controller.abort())
  onSignal('SIGTERM', () => controller.abort())
  return loader.dispatch(root, args, {
    stdin: stdinStream(),
    stdout: stdoutWriter(),
    stderr: stderrWriter(),
    cwd: cwd(),
    env: { ...env },
    signal: controller.signal,
  })
}

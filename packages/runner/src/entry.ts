// The entry of the tinyjs bundle (`docs/demi-next/tinyjs.md` § Entry modes):
// one packed binary reached through symlinks, the mode chosen by the name it
// was invoked by. `demi-runner` is runner mode; any other name is a root
// command in command mode.
import { argv, createRunnerHost, env, exit, identity, stderrWriter } from './machine'
import { basenamePath } from '@demicodes/utils'
import { runCommandMode, stateDir } from './command-mode'
import { RunnerMode } from './runner-mode'
import { RunnerState } from './state'

const RUNNER_NAME = 'demi-runner'

async function main(): Promise<number> {
  const name = basenamePath(argv[0] ?? '')
  if (name === RUNNER_NAME) return runnerMain(argv.slice(1))
  return runCommandMode(name, argv.slice(1))
}

async function runnerMain(args: readonly string[]): Promise<number> {
  const stderr = stderrWriter()
  const usage = async () => {
    await stderr('Usage: demi-runner run [--backend <url>]\n')
    return 2
  }
  if (args[0] !== 'run') return usage()
  let backendUrl: string | null = null
  for (let index = 1; index < args.length; index += 1) {
    if (args[index] === '--backend' && args[index + 1]) {
      backendUrl = args[index + 1]!
      index += 1
    } else {
      return usage()
    }
  }
  const dir = stateDir()
  const host = createRunnerHost()
  backendUrl ??= (await new RunnerState(host.fs, dir).readConfig())?.backendUrl ?? null
  if (!backendUrl) {
    await stderr('No backend URL: pass --backend <url> on first start.\n')
    return 2
  }
  // The root symlinks point at this file; jobs find them first in PATH.
  const executable = argv[0]!.includes('/') ? await host.fs.realpath(argv[0]!) : argv[0]!
  const runner = new RunnerMode({
    backendUrl,
    stateDir: dir,
    executable,
    ...(env.DEMI_RUNNER_NAME ? { name: env.DEMI_RUNNER_NAME } : {}),
    deviceEnv: { PATH: env.PATH ?? '/usr/bin:/bin', HOME: identity.homeDir },
    reconnect: env.DEMI_RUNNER_RECONNECT_MS ? { initialDelayMs: Number(env.DEMI_RUNNER_RECONNECT_MS) } : undefined,
  })
  return (await runner.run()) === 'rejected' ? 1 : 0
}

exit(await main())

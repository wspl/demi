// The entry of the tinyjs bundle (`docs/demi-next/tinyjs.md` § Entry modes):
// one packed binary reached through symlinks, the mode chosen by the name it
// was invoked by. `demi-runner` is runner mode; any other name is a root
// command in command mode.
import { argv, createRunnerHost, env, exit, identity, onSignal, pid, stderrWriter } from './machine'
import { basenamePath, errorMessage } from '@demicodes/utils'
import { runCommandMode, stateDir } from './command-mode'
import { GUEST_USER, bootGuest } from './init/boot'
import { RunnerMode } from './runner-mode'
import { RunnerState } from './state'

const RUNNER_NAME = 'demi-runner'

async function main(): Promise<number> {
  // The kernel started this binary as init: a managed guest (`managed-hosts.md` § Lifecycle).
  if (pid === 1) return initMain()
  const name = basenamePath(argv[0] ?? '')
  if (name === RUNNER_NAME) return runnerMain(argv.slice(1))
  return runCommandMode(name, argv.slice(1))
}

/** PID 1: the init duties, then the runner as a managed host with the guest user for every job; exiting is the VM's death. */
async function initMain(): Promise<number> {
  const stderr = stderrWriter()
  // Every line reaches the serial console before PID 1 exits: an exit here is the VM's death, and the console is its only trace.
  const pending: Promise<void>[] = []
  const log = (line: string) => void pending.push(Promise.resolve(stderr(`${line}\n`)).catch(() => {}))
  const flush = () => Promise.all(pending.splice(0))
  const host = createRunnerHost()
  let boot
  try {
    boot = await bootGuest(host, log)
  } catch (error) {
    log(`init failed: ${errorMessage(error)}`)
    await flush()
    return 1
  }
  const runner = new RunnerMode({
    backendUrl: boot.config.backendUrl,
    stateDir: boot.stateDir,
    executable: '/demi-runner',
    name: identity.hostname,
    // The guest's login table: the same values the backend names for the hostless shell (`sessions-and-targets.md` § What moves).
    deviceEnv: { PATH: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin', HOME: GUEST_USER.homeDir, USER: GUEST_USER.name, SHELL: '/bin/bash', LANG: 'C' },
    managed: true,
    deviceToken: boot.config.deviceToken,
    guest: { identity: { uid: GUEST_USER.uid, gid: GUEST_USER.gid, hostname: identity.hostname, homeDir: GUEST_USER.homeDir }, runAs: { uid: GUEST_USER.uid, gid: GUEST_USER.gid } },
    home: boot.home,
    log,
  })
  onSignal('SIGTERM', () => void runner.stop())
  const outcome = await runner.run()
  await flush()
  return outcome === 'rejected' ? 1 : 0
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
    // Jobs run in the environment the runner was started with: the device user's own.
    deviceEnv: { PATH: '/usr/bin:/bin', HOME: identity.homeDir, ...env },
    ...(env.DEMI_RUNNER_MANAGED ? { managed: true } : {}),
    reconnect: env.DEMI_RUNNER_RECONNECT_MS ? { initialDelayMs: Number(env.DEMI_RUNNER_RECONNECT_MS) } : undefined,
  })
  return (await runner.run()) === 'rejected' ? 1 : 0
}

exit(await main())

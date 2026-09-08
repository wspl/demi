import { manageRunner } from './management'
// The standalone runner entry. The separate native client owns root-command invocation.
import { argv, createRunnerHost, dropPrivileges, env, exit, identity, onSignal, pid, stderrWriter } from './machine'
import { dirnamePath, errorMessage } from '@demicodes/utils'
import { GUEST_USER, bootGuest } from './init/boot'
import { RunnerMode } from './runner-mode'
import { RunnerState } from './state'

async function main(): Promise<number> {
  // The kernel started this binary as init: a managed guest (`managed-hosts.md` § Lifecycle).
  if (pid === 1) return initMain()
  return runnerMain(argv.slice(1))
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
  const guestIdentity = dropPrivileges(GUEST_USER.uid, GUEST_USER.gid)
  const runner = new RunnerMode({
    backendUrl: boot.config.backendUrl,
    stateDir: boot.stateDir,
    clientExecutable: '/usr/bin/demi',
    name: identity.hostname,
    // Files, jobs, and commands all use the same guest account.
    deviceEnv: {
      PATH: '/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
      HOME: GUEST_USER.homeDir,
      USER: GUEST_USER.name,
      SHELL: '/bin/bash',
      LANG: 'C',
    },
    managed: true,
    deviceToken: boot.config.deviceToken,
    identity: { ...guestIdentity, homeDir: GUEST_USER.homeDir },
    volumes: boot.volumes,
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
    await stderr('Usage: demi-runner <run|status|drain> [--backend <url>]\n')
    return 2
  }
  const action = args[0]
  if (action !== 'run' && action !== 'status' && action !== 'drain') return usage()
  let backendUrl: string | null = null
  for (let index = 1; index < args.length; index += 1) {
    if (args[index] === '--backend' && args[index + 1]) {
      backendUrl = args[index + 1]!
      index += 1
    } else {
      return usage()
    }
  }
  if (!backendUrl && !env.DEMI_HOME) {
    await stderr('No backend URL: pass --backend <url>.\n')
    return 2
  }
  const hash = backendUrl ? await backendInstanceId(backendUrl) : ''
  const dir = env.DEMI_HOME ?? `${identity.homeDir}/.demi/instances/${hash}`
  if (action !== 'run') return manageRunner(dir, action)
  const host = createRunnerHost()
  backendUrl ??= (await new RunnerState(host.fs, dir).readConfig())?.backendUrl ?? null
  if (!backendUrl) {
    await stderr('No backend URL: pass --backend <url> on first start.\n')
    return 2
  }
  // The matching native client is installed beside the runner.
  const executable = argv[0]!.includes('/') ? await host.fs.realpath(argv[0]!) : argv[0]!
  const runner = new RunnerMode({
    backendUrl,
    stateDir: dir,
    clientExecutable: `${dirnamePath(executable)}/demi`,
    ...(env.DEMI_RUNNER_NAME ? { name: env.DEMI_RUNNER_NAME } : {}),
    // Jobs run in the environment the runner was started with: the device user's own.
    deviceEnv: { PATH: '/usr/bin:/bin', HOME: identity.homeDir, ...env },
    ...(env.DEMI_RUNNER_MANAGED ? { managed: true } : {}),
    reconnect: env.DEMI_RUNNER_RECONNECT_MS ? { initialDelayMs: Number(env.DEMI_RUNNER_RECONNECT_MS) } : undefined,
  })
  return (await runner.run()) === 'rejected' ? 1 : 0
}

/** Canonical backend URLs select stable, installation-local state directories. */
async function backendInstanceId(backendUrl: string): Promise<string> {
  const url = new URL(backendUrl).toString()
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(url))
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('')
}

try {
  exit(await main())
} catch (error) {
  await stderrWriter()(`demi-runner: ${errorMessage(error)}\n`)
  exit(1)
}

/**
 * The whole product for the preview laboratory's suite, on ports and in a
 * directory of its own (`scenarios.md` § Browser suite): the development
 * backend as `xtask dev` starts it, with its preview domain service and its
 * echo model, which calls no vendor, the web app's development server, and a runner paired to the
 * backend as a device. Each process leads a process group of its own, which
 * `stop` ends whole. The programs come from the directory
 * `DEMI_TEST_PROGRAMS` names, which `bun run test` builds; nothing here
 * builds one.
 */
import { mkdir, readFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { startGroup, stopGroup, type Started } from '../processes'

/** How long the backend may take to answer and seed its account; the web app to answer; a runner to print its code. */
const BACKEND_MS = 120_000
const WEB_MS = 180_000
const RUNNER_MS = 60_000
/** How long a process group may take to stop before it is killed. */
const STOP_MS = 15_000
const POLL_MS = 200

/** The line `xtask dev` prints once the backend answers and its account is seeded. */
const BACKEND_READY = /^The development backend serves at /m
/** What a runner prints before its pairing code. */
const PAIRING_CODE = /^demi-runner: pairing code: (\S+)$/m

/** The account `xtask dev` seeds when no `DEMI_DEV_*` variable names one (`backend.md` § One-command development backend). */
export const ACCOUNT = { email: 'developer@example.test', password: 'development' }

const repositoryRoot = resolve(import.meta.dir, '../../../..')

export interface Stack {
  /** Where the web app is, which the browser loads the product from. */
  web: string
  /** The paired runner's pairing code and its working directory. */
  runner: { code: string; home: string }
  stop(): Promise<void>
}

/** The built program `name`, from the directory `DEMI_TEST_PROGRAMS` names. */
function program(name: string): string {
  const directory = process.env.DEMI_TEST_PROGRAMS
  if (!directory) {
    throw new Error('Set DEMI_TEST_PROGRAMS to the built test programs (bun run test builds them into target/debug)')
  }
  return join(resolve(repositoryRoot, directory), name)
}

/** A port nothing listens on now: `xtask dev` and Vite take a port number, not 0. */
export function freePort(): number {
  const probe = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } })
  const port = probe.port
  probe.stop(true)
  return port
}

/**
 * The environment of a program the suite starts: the test process's own
 * without its `DEMI_` variables, which name the test's resources, not the
 * program's settings, and which the backend and the runner refuse.
 */
function environment(extra: Record<string, string> = {}): Record<string, string | undefined> {
  return { ...Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('DEMI_'))), ...extra }
}

/** Waits until `ready` answers something, failing when the process ends first or `ms` passes. */
async function untilReady<T>(what: string, started: Started, ready: () => Promise<T | null>, ms: number): Promise<T> {
  let exit: number | null = null
  void started.exited.then((code) => {
    exit = code
  })
  const deadline = Date.now() + ms
  for (;;) {
    const answer = await ready()
    if (answer !== null) {
      return answer
    }
    const log = async () => (await readFile(started.group.log, 'utf8')).split('\n').slice(-30).join('\n')
    if (exit !== null) {
      throw new Error(`${what} exited with ${exit} before it was ready:\n${await log()}`)
    }
    if (Date.now() > deadline) {
      throw new Error(`${what} was not ready within ${ms / 1000} s:\n${await log()}`)
    }
    await Bun.sleep(POLL_MS)
  }
}

/** The first match of `pattern` in a process's log, or null. */
async function logged(started: Started, pattern: RegExp): Promise<RegExpExecArray | null> {
  return pattern.exec(await readFile(started.group.log, 'utf8'))
}

/** Starts the product under `root`, a temporary directory the caller removes after `stop`. */
export async function startStack(root: string): Promise<Stack> {
  const started: Started[] = []
  const stop = async () => {
    // The runner first, then the web app, then the backend with its preview domain service.
    for (const each of [...started].reverse()) {
      await stopGroup(each.group, STOP_MS)
    }
  }
  try {
    const backendPort = freePort()
    const webPort = freePort()
    const web = `http://127.0.0.1:${webPort}`
    const backend = startGroup(
      [
        program('xtask'), 'dev', '--port', String(backendPort), '--data', join(root, 'backend'),
        '--preview-port', String(freePort()), '--web-origin', web,
      ],
      // The echo model, which answers without a vendor: the conversation needs a model to open.
      { cwd: repositoryRoot, env: environment({ DEMI_DEV_ECHO: '1' }), log: join(root, 'backend.log') },
    )
    started.push(backend)
    await untilReady('The backend', backend, () => logged(backend, BACKEND_READY), BACKEND_MS)

    const page = startGroup(
      [process.execPath, 'scripts/web-dev.ts', '--port', String(webPort), '--strictPort'],
      { cwd: repositoryRoot, env: environment({ DEMI_BACKEND_URL: `http://127.0.0.1:${backendPort}` }), log: join(root, 'web.log') },
    )
    started.push(page)
    await untilReady('The web app', page, async () => {
      const answer = await fetch(web, { signal: AbortSignal.timeout(2_000) }).catch(() => null)
      return answer?.ok ? true : null
    }, WEB_MS)

    // A device of its own, as a person's laptop is: its home and its state in the suite's directory.
    const home = join(root, 'runner-home')
    await mkdir(home, { recursive: true })
    const runner = startGroup(
      [program('demi-runner'), 'run', '--backend', `http://127.0.0.1:${backendPort}`],
      {
        cwd: home,
        env: environment({ HOME: home, USERPROFILE: home, DEMI_HOME: join(root, 'runner-state'), DEMI_RUNNER_NAME: 'preview-lab' }),
        log: join(root, 'runner.log'),
      },
    )
    started.push(runner)
    const code = (await untilReady('The runner', runner, () => logged(runner, PAIRING_CODE), RUNNER_MS))[1]!
    return { web, runner: { code, home }, stop }
  } catch (error) {
    await stop()
    throw error
  }
}

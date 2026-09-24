import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import type { Subprocess, WebSocketOptions } from 'bun'

// The browser-contract suite's world (`scenarios.md` § Browser-contract
// suite): the backend executable on a data directory of its own, real
// runners, and a browser for the web application's API client and the agent
// socket, which carries the session cookie as a page's requests do.

/** How long a process may take to start: the backend to answer, a runner to print its code. */
const START_MS = 15_000
/** How long a process may take to stop before it is killed. */
const STOP_MS = 5_000
/** What a runner prints before its pairing code. */
const PAIRING_CODE = 'demi-runner: pairing code: '

const repositoryRoot = resolve(import.meta.dir, '../../../../..')

/**
 * A program the tests start, from the directory `DEMI_TEST_PROGRAMS` names.
 * The test script builds the programs first (`bun run test`); a test never
 * builds one. (`@demicodes/host-remote`'s locator reads the same variable;
 * `web` may not depend on it, and it leaves with the TypeScript backend.)
 */
function testProgram(name: string): string {
  const directory = process.env.DEMI_TEST_PROGRAMS
  if (!directory) {
    throw new Error('Set DEMI_TEST_PROGRAMS to the built test programs: cargo build --workspace --all-targets --features demi-runner/test-fixtures, then DEMI_TEST_PROGRAMS=target/debug (bun run test does both)')
  }
  return join(resolve(repositoryRoot, directory), name)
}

/** A port nothing listens on now: the backend takes a port number, not 0. */
function freePort(): number {
  const probe = Bun.listen({ hostname: '127.0.0.1', port: 0, socket: { data() {} } })
  const port = probe.port
  probe.stop(true)
  return port
}

/** Collects a process's output lines, and hands each to `line` as it arrives. */
async function readLines(stream: ReadableStream<Uint8Array>, line: (text: string) => void): Promise<void> {
  const decoder = new TextDecoder()
  let pending = ''
  for await (const chunk of stream) {
    pending += decoder.decode(chunk, { stream: true })
    const lines = pending.split('\n')
    pending = lines.pop() ?? ''
    for (const each of lines) {
      line(each)
    }
  }
  if (pending) {
    line(pending)
  }
}

/** Asks a process to stop and waits for it, killing it when it does not stop in time. */
async function terminate(child: Subprocess): Promise<void> {
  child.kill('SIGTERM')
  const stopped = await Promise.race([child.exited.then(() => true), Bun.sleep(STOP_MS).then(() => false)])
  if (!stopped) {
    child.kill('SIGKILL')
    await child.exited
  }
}

/** The backend executable, serving on a port of its own until `stop`. */
export interface Backend {
  /** Where the page loads from: the API is under `/api`. */
  origin: string
  /** What the backend logged, for a failure's message. */
  log(): string
  stop(): Promise<void>
}

/**
 * Starts the backend executable on a new data directory under `root`. It
 * has no machine manager and publishes no native command release: a
 * conversation reaches a Host only on a paired runner.
 */
export async function startBackend(root: string): Promise<Backend> {
  const data = join(root, 'backend')
  const port = freePort()
  const origin = `http://127.0.0.1:${port}`
  const native = join(root, 'native.json')
  // No release to publish, so the store is never asked.
  await writeFile(native, JSON.stringify({ releases: [], store: { provider: 's3', bucket: 'demi-contract', region: 'us-east-1' } }))
  const lines: string[] = []
  const child = Bun.spawn([testProgram('demi-backend')], {
    env: {
      ...process.env,
      DEMI_BACKEND_DATA: data,
      DEMI_BACKEND_PORT: String(port),
      DEMI_INSTANCE_MODE: 'shared',
      DEMI_BACKEND_PUBLIC_URL: origin,
      DEMI_MACHINES_SOCKET: join(root, 'machines.sock'),
      DEMI_NATIVE_CONFIG: native,
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const reading = Promise.all([readLines(child.stdout, (line) => lines.push(line)), readLines(child.stderr, (line) => lines.push(line))])
  const log = () => lines.join('\n')
  const deadline = Date.now() + START_MS
  for (;;) {
    if (child.exitCode !== null) {
      await reading
      throw new Error(`The backend exited with ${child.exitCode}:\n${log()}`)
    }
    try {
      const answer = await fetch(`${origin}/api/setup`)
      if (answer.ok) {
        break
      }
    } catch {
      // Not listening yet.
    }
    if (Date.now() > deadline) {
      await terminate(child)
      throw new Error(`The backend did not answer within ${START_MS} ms:\n${log()}`)
    }
    await Bun.sleep(50)
  }
  return {
    origin,
    log,
    async stop() {
      await terminate(child)
      await reading
    },
  }
}

/** A runner of a device, started, until `stop`. */
export interface Runner {
  /** The device's home, where its Hosts start work. */
  home: string
  /** The pairing code it printed, for the page to claim. */
  code: Promise<string>
  log(): string
  stop(): Promise<void>
}

/** Starts the real runner for the backend at `origin`, as a device named `name` whose files live under `root`. */
export async function startRunner(root: string, origin: string, name: string): Promise<Runner> {
  const home = join(root, `${name}-home`)
  await mkdir(home, { recursive: true })
  // The runner keeps the test run's temporary directory, which the test
  // script removes: its command socket lives there, and a deeper path would
  // exceed the length a Unix socket's path may have.
  const env: Record<string, string | undefined> = {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    DEMI_HOME: join(root, `${name}-state`),
    DEMI_RUNNER_NAME: name,
  }
  // A paired device, not a managed Cloud guest.
  delete env.DEMI_RUNNER_MANAGED
  const child = Bun.spawn([testProgram('demi-runner'), 'run', '--backend', origin], {
    cwd: home,
    env,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const lines: string[] = []
  const printed = Promise.withResolvers<string>()
  const line = (text: string) => {
    lines.push(text)
    if (text.startsWith(PAIRING_CODE)) {
      printed.resolve(text.slice(PAIRING_CODE.length).trim())
    }
  }
  const reading = Promise.all([readLines(child.stdout, line), readLines(child.stderr, line)])
  const log = () => lines.join('\n')
  const timeout = setTimeout(() => printed.reject(new Error(`The runner printed no pairing code within ${START_MS} ms:\n${log()}`)), START_MS)
  const code = printed.promise.finally(() => clearTimeout(timeout))
  // A test that does not wait for the code must not leave its rejection unhandled.
  code.catch(() => {})
  return {
    home,
    code,
    log,
    async stop() {
      clearTimeout(timeout)
      await terminate(child)
      await reading
    },
  }
}

/**
 * The global WebSocket's constructor as Bun declares it: Bun's client takes
 * `{ headers }`, but with the DOM library loaded, bun-types give the global
 * the DOM's type, whose constructor takes only protocols.
 */
type BunWebSocketConstructor = typeof WebSocket & (new (url: string | URL, options: WebSocketOptions) => WebSocket)

/**
 * The page's browser for the web application's modules: `fetch` resolves the
 * page's relative `/api` paths against `origin` and keeps the cookies its
 * answers set, and a `WebSocket` sends them and the page's origin with its
 * upgrade, as a browser does for a same-origin page. `restore` puts the
 * test process's own back.
 */
export function openBrowser(origin: string) {
  const cookies = new Map<string, string>()
  const realFetch = globalThis.fetch
  const RealWebSocket = globalThis.WebSocket as BunWebSocketConstructor

  const cookieHeader = () => [...cookies].map(([name, value]) => `${name}=${value}`).join('; ')

  function keep(answer: Response): void {
    for (const cookie of answer.headers.getSetCookie()) {
      const [pair = '', ...attributes] = cookie.split(';')
      const separator = pair.indexOf('=')
      const name = pair.slice(0, separator).trim()
      const value = pair.slice(separator + 1).trim()
      const removed = attributes.some((attribute) => /^\s*max-age\s*=\s*0\s*$/i.test(attribute))
      if (removed || !value) {
        cookies.delete(name)
      } else {
        cookies.set(name, value)
      }
    }
  }

  async function pageFetch(input: string | URL | Request, init?: RequestInit): Promise<Response> {
    const request = new Request(new URL(input instanceof Request ? input.url : String(input), origin), input instanceof Request ? input : init)
    if (cookies.size) {
      request.headers.set('Cookie', cookieHeader())
    }
    const answer = await realFetch(request)
    keep(answer)
    return answer
  }

  class PageWebSocket extends RealWebSocket {
    constructor(url: string | URL) {
      super(url, { headers: { Cookie: cookieHeader(), Origin: origin } })
    }
  }

  globalThis.fetch = Object.assign(pageFetch, { preconnect: realFetch.preconnect })
  globalThis.WebSocket = PageWebSocket

  return {
    /** The URL of a WebSocket route under `/api`. */
    socketUrl: (path: string): string => new URL(`/api${path}`, origin.replace(/^http/, 'ws')).toString(),
    /** Whether the page holds a session cookie. */
    signedIn: (): boolean => cookies.size > 0,
    restore(): void {
      globalThis.fetch = realFetch
      globalThis.WebSocket = RealWebSocket
    },
  }
}

/** A temporary directory for one suite's processes, which `remove` deletes. */
export async function temporaryRoot(): Promise<{ path: string; remove(): Promise<void> }> {
  const path = await mkdtemp(join(tmpdir(), 'demi-contract-'))
  return { path, remove: () => rm(path, { recursive: true, force: true }) }
}

// `bun browse` (browse.md § A call): hands a script, read from standard
// input or from the file named, to the slot's tool server, starting the
// server when none answers, and prints what the call prints. `up`, `down`
// and `stop` are the helpers of the same names as a script of one line;
// `help` answers here, without a server.
import { closeSync, existsSync, mkdirSync, openSync, readFileSync } from 'node:fs'
import net from 'node:net'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { parseArgs } from 'node:util'
import { HELP } from './demi/help'
import { replySchema, type Request } from './protocol'
import { currentSlot, slotPaths } from './slot'

/** How long a server that is starting may take to listen. */
const SERVER_START_MS = 15_000
/** How long a call may run unless `--limit` says otherwise: ten minutes. */
const LIMIT_MS = 10 * 60 * 1000

const slot = currentSlot()
const paths = slotPaths(slot)

let parsed
try {
  parsed = parseArgs({
    args: Bun.argv.slice(2),
    options: {
      headed: { type: 'boolean' },
      limit: { type: 'string' },
      wipe: { type: 'boolean' },
      help: { type: 'boolean' },
    },
    allowPositionals: true,
  })
} catch (error) {
  fail(`${error instanceof Error ? error.message : String(error)}; bun browse help says how to call it`)
}
const { values, positionals } = parsed
const [first, ...rest] = positionals

if (values.help || first === 'help') {
  console.log(HELP)
  process.exit(0)
}

const script = scriptOf()
const request: Request = {
  script: script.text,
  file: script.file,
  headed: values.headed ?? false,
  env: callerEnv(),
  limitMs: values.limit === undefined ? LIMIT_MS : limitOf(values.limit),
}
let exitCode = await ask()
if (exitCode === 'restart') {
  exitCode = await ask()
}
if (exitCode === 'restart' || exitCode === null) {
  fail(`The slot's tool server ended during the call; its log is ${paths.daemonLog}. The next call starts it again.`)
}
process.exit(exitCode)

/** The script the command line asks for, and the file it names it by. */
function scriptOf(): { text: string, file: string | null } {
  const servers = rest.map((name) => JSON.stringify(name))
  switch (first) {
    case 'up':
      return { text: `await demi.up(${servers.join(', ')})`, file: null }
    case 'down':
      if (values.wipe) {
        servers.push('{ wipe: true }')
      }
      return { text: `await demi.down(${servers.join(', ')})`, file: null }
    case 'stop':
      return { text: 'await demi.stop()', file: null }
    case undefined:
      if (process.stdin.isTTY) {
        fail(`bun browse reads a script from standard input or a file.\n\n${HELP}`)
      }
      return { text: readFileSync(0, 'utf8'), file: null }
    default:
      if (rest.length > 0 || !existsSync(first)) {
        fail(`bun browse runs a script file, or up, down, stop or help, not ${positionals.join(' ')}`)
      }
      return { text: readFileSync(first, 'utf8'), file: first }
  }
}

/** The caller's `DEMI_*` variables, which the processes the tool starts get. */
function callerEnv(): Record<string, string> {
  const env: Record<string, string> = {}
  for (const [name, value] of Object.entries(process.env)) {
    if (name.startsWith('DEMI_') && value !== undefined) {
      env[name] = value
    }
  }
  return env
}

/** `--limit` in milliseconds: seconds, or minutes with an `m`. */
function limitOf(text: string): number {
  const limit = /^(\d+(?:\.\d+)?)(s|m)?$/.exec(text)
  if (!limit) {
    fail(`--limit takes seconds or minutes, such as 90 or 15m, not ${text}`)
  }
  return Math.round(Number(limit[1]) * (limit[2] === 'm' ? 60_000 : 1_000))
}

function fail(message: string): never {
  console.error(message)
  process.exit(1)
}

/**
 * Sends the call to the server and prints what it answers. Answers the
 * exit code, `restart` when the server ended without running it, or null
 * when the server ended during it.
 */
async function ask(): Promise<number | 'restart' | null> {
  const socket = await connectToServer()
  socket.write(`${JSON.stringify(request)}\n`)
  let answer: number | 'restart' | null = null
  for await (const line of createInterface({ input: socket })) {
    const reply = replySchema.parse(JSON.parse(line))
    if ('out' in reply) {
      console.log(reply.out)
    } else if ('err' in reply) {
      console.error(reply.err)
    } else if ('restart' in reply) {
      answer = 'restart'
    } else {
      answer = reply.exit
    }
  }
  return answer
}

/** A connection to the slot's server, started first when none answers. */
async function connectToServer(): Promise<net.Socket> {
  const existing = await connect()
  if (existing) {
    return existing
  }
  startServer()
  const deadline = Date.now() + SERVER_START_MS
  while (Date.now() < deadline) {
    const socket = await connect()
    if (socket) {
      return socket
    }
    await Bun.sleep(50)
  }
  fail(`The slot's tool server did not start within ${SERVER_START_MS / 1000} s; its log is ${paths.daemonLog}.`)
}

function connect(): Promise<net.Socket | null> {
  return new Promise((resolve) => {
    const socket = net.connect(paths.socket)
    socket.once('connect', () => {
      socket.removeAllListeners('error')
      resolve(socket)
    })
    socket.once('error', () => resolve(null))
  })
}

function startServer(): void {
  mkdirSync(slot.folder, { recursive: true })
  const log = openSync(paths.daemonLog, 'a')
  try {
    // The workspace's packages resolve to their sources, as in the tests.
    const server = Bun.spawn([process.execPath, '--conditions', 'development', join(import.meta.dir, 'daemon.ts')], {
      cwd: slot.root,
      stdio: ['ignore', log, log],
      // Its own session: the server outlives this command and its terminal.
      detached: true,
    })
    server.unref()
  } finally {
    closeSync(log)
  }
}

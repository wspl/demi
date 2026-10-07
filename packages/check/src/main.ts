// `bun check <command>` (product-checks.md): hands the command line to the
// slot's daemon, starting the daemon when none answers, and prints what the
// command prints. `help` answers here, without a daemon.
import { mkdirSync, openSync, closeSync } from 'node:fs'
import net from 'node:net'
import { createInterface } from 'node:readline'
import { join } from 'node:path'
import { findCommand, helpText } from './commands'
import { replySchema, type Request } from './protocol'
import { currentSlot, slotPaths } from './slot'

/** How long a daemon that is starting may take to listen. */
const DAEMON_START_MS = 15_000

const slot = currentSlot()
const paths = slotPaths(slot)
const argv = Bun.argv.slice(2)
const headed = argv.includes('--headed')
const commandLine = argv.filter((word) => word !== '--headed')

// `help <command>`, and `<command> --help` as every command line tool takes it.
const asksHelp = commandLine.length === 0 || commandLine[0] === 'help' || commandLine.includes('--help')
if (asksHelp) {
  const name = commandLine[0] === 'help' || commandLine[0] === '--help' ? commandLine[1] : commandLine[0]
  const command = name === undefined ? undefined : findCommand(name)
  console.log(command ? `bun check ${command.usage}\n  ${command.summary}` : helpText())
  process.exit(name !== undefined && !command ? 1 : 0)
}

const env: Record<string, string> = {}
for (const [name, value] of Object.entries(process.env)) {
  if (name.startsWith('DEMI_') && value !== undefined) {
    env[name] = value
  }
}
const request: Request = { argv: commandLine, headed, env }
let exitCode = await ask()
if (exitCode === 'restart') {
  exitCode = await ask()
}
if (exitCode === 'restart' || exitCode === null) {
  console.error(`The slot's daemon ended during the command; its log is ${paths.daemonLog}. The next command starts it again.`)
  process.exit(1)
}
process.exit(exitCode)

/**
 * Sends the command to the daemon and prints what it answers. Answers the
 * exit code, `restart` when the daemon ended without running it, or null
 * when the daemon ended during it.
 */
async function ask(): Promise<number | 'restart' | null> {
  const socket = await connectToDaemon()
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

/** A connection to the slot's daemon, started first when none answers. */
async function connectToDaemon(): Promise<net.Socket> {
  const existing = await connect()
  if (existing) {
    return existing
  }
  startDaemon()
  const deadline = Date.now() + DAEMON_START_MS
  while (Date.now() < deadline) {
    const socket = await connect()
    if (socket) {
      return socket
    }
    await Bun.sleep(50)
  }
  console.error(`The slot's daemon did not start within ${DAEMON_START_MS / 1000} s; its log is ${paths.daemonLog}.`)
  process.exit(1)
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

function startDaemon(): void {
  mkdirSync(slot.folder, { recursive: true })
  const log = openSync(paths.daemonLog, 'a')
  try {
    // The workspace's packages resolve to their sources, as in the tests.
    const daemon = Bun.spawn([process.execPath, '--conditions', 'development', join(import.meta.dir, 'daemon.ts')], {
      cwd: slot.root,
      stdio: ['ignore', log, log],
      // Its own session: the daemon outlives this command and its terminal.
      detached: true,
    })
    daemon.unref()
  } finally {
    closeSync(log)
  }
}

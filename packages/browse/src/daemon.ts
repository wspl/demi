// The slot's daemon, the tool's server (browse.md § One browser per slot):
// it attaches to the slot's browser, holds the slot's network between
// commands and runs each command it receives on its socket in
// `.cache/browse/`. When the tool's code changes it ends and leaves the
// browser as it is, for the next daemon, with the new code, to attach to.
// It ends with the browser on `bun browse stop` or `down`, or after 30
// minutes without a command; the next command starts it again, and so one
// that crashed.
import { existsSync, mkdirSync, readdirSync, statSync, unlinkSync } from 'node:fs'
import { join } from 'node:path'
import net from 'node:net'
import { createInterface } from 'node:readline'
import { Browser } from './browser'
import { CommandFailure, type Context } from './command'
import { findCommand } from './commands'
import { Network } from './network'
import { requestSchema, type Reply } from './protocol'
import { currentSlot, slotPaths } from './slot'
import { readState, updateState } from './state'

/** How long the daemon waits without a command before it ends. */
const IDLE_MS = 30 * 60 * 1000

const slot = currentSlot()
const paths = slotPaths(slot)
mkdirSync(slot.folder, { recursive: true })

// One daemon per slot: a second one started meanwhile leaves.
if (await answers(paths.socket)) {
  process.exit(0)
}
if (existsSync(paths.socket)) {
  // A crashed daemon's socket, which nobody answers on.
  unlinkSync(paths.socket)
}

/** When the tool's code last changed as this daemon started; a later change ends it. */
const loadedCode = newestSource(import.meta.dir)

let progress: (line: string) => void = (line) => console.log(line)
const browser = new Browser(slot, (line) => progress(line))
let network: Promise<Network> | null = null
let running = 0
let idle: ReturnType<typeof setTimeout> | null = null
let ending = false

const server = net.createServer((socket) => void serve(socket))
server.listen(paths.socket, () => console.log(`slot ${slot.number}: daemon ${process.pid} listens on ${paths.socket}`))
waitIdle()
// The network listens from the start: a page the browser still shows
// reaches the web app through it again before any command asks for it.
openNetwork().catch((error) => console.error(`the network does not listen: ${error}`))
// A browser the last daemon left is attached to at once, so that its page's
// logs go on from where that daemon left them.
browser.attachRunning().catch((error) => console.error(`the browser could not be attached to: ${error}`))
process.on('SIGTERM', () => void end('with the browser'))
process.on('SIGINT', () => void end('with the browser'))

/** The newest modification time of the files under `folder`, in milliseconds. */
function newestSource(folder: string): number {
  let newest = 0
  for (const entry of readdirSync(folder, { withFileTypes: true, recursive: true })) {
    if (entry.isFile()) {
      newest = Math.max(newest, statSync(join(entry.parentPath, entry.name)).mtimeMs)
    }
  }
  return newest
}

/** Whether a daemon answers on `path`. */
function answers(path: string): Promise<boolean> {
  return new Promise((resolve) => {
    const probe = net.connect(path)
    probe.once('connect', () => {
      probe.destroy()
      resolve(true)
    })
    probe.once('error', () => resolve(false))
  })
}

function waitIdle(): void {
  if (idle) {
    clearTimeout(idle)
  }
  idle = setTimeout(() => {
    if (running === 0) {
      console.log('30 minutes without a command: ending')
      void end('with the browser')
    }
  }, IDLE_MS)
}

async function serve(socket: net.Socket): Promise<void> {
  const lines = createInterface({ input: socket })
  const first = await new Promise<string | null>((resolve) => {
    lines.once('line', resolve)
    lines.once('close', () => resolve(null))
  })
  lines.close()
  if (first === null) {
    socket.destroy()
    return
  }
  const send = (reply: Reply) => {
    if (!socket.destroyed) {
      socket.write(`${JSON.stringify(reply)}\n`)
    }
  }
  if (newestSource(import.meta.dir) > loadedCode) {
    // A command would run the code this daemon loaded, not the code as it
    // is now: the caller asks a new daemon instead, which can listen once
    // this one no longer does, and which attaches to the same browser.
    stopListening()
    // The next daemon listens on the network's port as it starts, and goes
    // on from the page's logs as it attaches to the browser.
    await closeNetwork()
    await browser.leave()
    send({ restart: true })
    socket.end()
    console.log('the tool\'s code changed: ending for a new daemon, leaving the browser')
    await end('leaving the browser')
    return
  }
  running += 1
  waitIdle()
  let exit = 0
  let endAfter = false
  try {
    const request = requestSchema.parse(JSON.parse(first))
    const print = (line: string) => send({ out: line })
    progress = print
    if (request.headed) {
      browser.showWindow()
    }
    const context: Context = {
      slot,
      browser,
      network: openNetwork,
      print,
      env: request.env,
      run: (argv) => runCommand(context, argv),
      endDaemon: () => {
        endAfter = true
      },
    }
    await runCommand(context, request.argv)
  } catch (error) {
    exit = 1
    if (error instanceof CommandFailure) {
      send({ err: error.message })
    } else if (error instanceof Error) {
      send({ err: error.message })
      console.error(error)
    } else {
      send({ err: String(error) })
    }
  } finally {
    running -= 1
    waitIdle()
  }
  send({ exit })
  socket.end()
  if (endAfter) {
    await end('with the browser')
  }
}

async function runCommand(context: Context, argv: string[]): Promise<void> {
  const [name, ...rest] = argv
  const command = name === undefined ? undefined : findCommand(name)
  if (!command) {
    throw new CommandFailure(`bun browse knows no command ${name ?? ''}; bun browse help lists them`)
  }
  const module = await command.load()
  await module.run(context, rest)
}

function openNetwork(): Promise<Network> {
  if (!network) {
    // The conditions `net` set, which a daemon that started again keeps.
    network = Network.listen(slot.ports.network, slot.ports.web, readState(slot).net)
    network.catch(() => {
      network = null
    })
  }
  return network
}

/** Takes no more commands: the next caller starts a new daemon. */
function stopListening(): void {
  if (!server.listening) {
    return
  }
  server.close()
  if (existsSync(paths.socket)) {
    unlinkSync(paths.socket)
  }
}

/**
 * Closes the network and ends the daemon, stopping the browser or leaving
 * it for the next daemon. A daemon that leaves it simply exits: its
 * connection to the browser ends with it, the browser's pages stay.
 */
async function end(how: 'with the browser' | 'leaving the browser'): Promise<void> {
  if (ending) {
    return
  }
  ending = true
  if (idle) {
    clearTimeout(idle)
  }
  // Once the socket and the network are closed, nothing Bun counts keeps
  // the process alive while the browser quits, and Bun would end it with
  // the browser still running: this timer holds it until it exits.
  setInterval(() => undefined, 1_000)
  stopListening()
  await closeNetwork()
  if (how === 'with the browser') {
    await browser.close()
    // `net`'s conditions are the browser's: the next browser starts online.
    updateState(slot, (state) => {
      delete state.net
    })
  }
  process.exit(0)
}

async function closeNetwork(): Promise<void> {
  const open = network
  network = null
  if (open) {
    // A network that never listened has nothing to close.
    await open.then((listening) => listening.shutdown(), () => undefined)
  }
}

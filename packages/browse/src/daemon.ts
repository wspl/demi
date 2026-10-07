// The slot's tool server (browse.md § One browser per slot): it attaches
// to the slot's browser, holds the slot's network and `keep` between calls,
// and runs each call it receives on its socket in `.cache/browse/`. When the
// tool's code changes it ends and leaves the browser, the network's
// conditions, the page's logs and `keep` for the next server, with the new
// code, to go on from; it does the same when a script runs into its time
// limit, since only the end of the process stops a script. It ends with the
// browser on `demi.stop()` or `demi.down()`, or after 30 minutes without a
// call; the next call starts it again, and so one that crashed.
import { existsSync, mkdirSync, readdirSync, rmSync, statSync, unlinkSync } from 'node:fs'
import { join } from 'node:path'
import net from 'node:net'
import { createInterface } from 'node:readline'
import { Browser } from './browser'
import { runCall } from './call'
import { restoreKeep, saveKeep } from './keep'
import { Network } from './network'
import { requestSchema, type Reply } from './protocol'
import { currentSlot, slotPaths } from './slot'
import { readState, updateState } from './state'

/** How long the server waits without a call before it ends. */
const IDLE_MS = 30 * 60 * 1000

const slot = currentSlot()
const paths = slotPaths(slot)
mkdirSync(slot.folder, { recursive: true })

// One server per slot: a second one started meanwhile leaves.
if (await answers(paths.socket)) {
  process.exit(0)
}
if (existsSync(paths.socket)) {
  // A crashed server's socket, which nobody answers on.
  unlinkSync(paths.socket)
}

/** When the tool's code last changed as this server started; a later change ends it. */
const loadedCode = newestSource(import.meta.dir)

let progress: (line: string) => void = (line) => console.log(line)
const browser = new Browser(slot, (line) => progress(line))
/** What the scripts keep between calls, as the last server left it. */
const keep = restoreKeep(paths.keep)
let network: Promise<Network> | null = null
let running = 0
let idle: ReturnType<typeof setTimeout> | null = null
let ending = false

const server = net.createServer((socket) => void serve(socket))
server.listen(paths.socket, () => console.log(`slot ${slot.number}: server ${process.pid} listens on ${paths.socket}`))
waitIdle()
// The network listens from the start: a page the browser still shows
// reaches the web app through it again before any call asks for it.
openNetwork().catch((error) => console.error(`the network does not listen: ${error}`))
// A browser the last server left is attached to at once, so that its page's
// logs go on from where that server left them.
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

/** Whether a server answers on `path`. */
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
      console.log('30 minutes without a call: ending')
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
    // A call would run the code this server loaded, not the code as it is
    // now: the caller asks a new server instead, which can listen once this
    // one no longer does, and which goes on from what this one leaves.
    await leave(send, 'the tool\'s code changed')
    send({ restart: true })
    socket.end()
    await end('leaving the browser')
    return
  }
  running += 1
  waitIdle()
  let exit = 1
  let after: 'stay' | 'end' | 'leave' = 'stay'
  try {
    const request = requestSchema.parse(JSON.parse(first))
    const print = (line: string) => send({ out: line })
    progress = print
    if (request.headed) {
      browser.showWindow()
    }
    const outcome = await runCall({ slot, browser, network: openNetwork, keep }, request, print)
    exit = outcome.exit
    after = outcome.overran ? 'leave' : outcome.endServer ? 'end' : 'stay'
  } catch (error) {
    // The call reports its script's failures itself: this is the tool's own.
    send({ err: `bun browse failed: ${error instanceof Error ? error.message : String(error)}; the server's log is ${paths.daemonLog}` })
    console.error(error)
  } finally {
    running -= 1
    waitIdle()
  }
  if (after === 'leave') {
    await leave(send, 'the script ran into its time limit')
  }
  send({ exit })
  socket.end()
  if (after === 'end') {
    await end('with the browser')
  } else if (after === 'leave') {
    await end('leaving the browser')
  }
}

/**
 * Takes no more calls and leaves the browser, the page's logs and `keep`
 * for the next server; tells the caller what of `keep` could not go along.
 */
async function leave(send: (reply: Reply) => void, why: string): Promise<void> {
  stopListening()
  // The next server listens on the network's port as it starts, and goes
  // on from the page's logs as it attaches to the browser.
  await closeNetwork()
  await browser.leave()
  for (const lost of saveKeep(paths.keep, keep)) {
    send({ out: `${lost}; it is gone as the tool's server restarts` })
  }
  console.log(`${why}: ending for a new server, leaving the browser`)
}

function openNetwork(): Promise<Network> {
  if (!network) {
    // The conditions `demi.net` set, which a server that started again keeps.
    network = Network.listen(slot.ports.network, slot.ports.web, readState(slot).net)
    network.catch(() => {
      network = null
    })
  }
  return network
}

/** Takes no more calls: the next caller starts a new server. */
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
 * Closes the network and ends the server, stopping the browser or leaving
 * it for the next server. A server that leaves it simply exits: its
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
    // `demi.net`'s conditions are the browser's: the next browser starts online.
    updateState(slot, (state) => {
      delete state.net
    })
    // What the scripts kept was of this browser's pages.
    rmSync(paths.keep, { force: true })
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

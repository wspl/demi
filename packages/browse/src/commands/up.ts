// `bun browse up [backend] [web] [gallery]` (browse.md § The slot's
// servers): starts what it names on the slot's ports, the backend and the
// web app when it names nothing, waits until each answers, and signs the
// browser in with the development account once the web app is up. The web
// app needs the backend, so naming it starts the backend too. Each server
// leads a process group the slot's state records for `down`. The backend
// keeps its data in the slot's folder, so it comes back with the account
// and the conversations it had until `down --wipe`.
import { copyFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { CommandFailure, parse, webBase, type Context } from '../command'
import { startGroup, type Started } from '../processes'
import {
  answersOk, childEnv, dotEnvValue, freshLog, lineOf, listening, runningServer, untilReady, userCheckout,
} from '../servers'
import { local, slotPaths } from '../slot'
import { readState, updateState, type ServerName } from '../state'

const USAGE = 'up [backend] [web] [gallery]'
const SERVERS = ['backend', 'web', 'gallery'] as const

/**
 * How long the backend may take: `xtask dev` builds the Cargo selection
 * first, about a minute and a half in a warm slot and much longer in a cold
 * one.
 */
const BACKEND_START_MS = 20 * 60 * 1000
/** How long the web app or the gallery may take: each generates the contracts first. */
const VITE_START_MS = 5 * 60 * 1000

/** The line `xtask dev` prints once the backend answers and its account and models are seeded. */
const BACKEND_READY = /^The development backend serves at /m
const ACCOUNT = /^\s*Account: (\S+), password (.+)$/m

export async function run(context: Context, argv: string[]): Promise<void> {
  const { positionals } = parse(argv, {}, USAGE)
  const unknown = positionals.filter((name) => !SERVERS.some((server) => server === name))
  if (unknown.length > 0) {
    throw new CommandFailure(`up starts backend, web and gallery, not ${unknown.join(', ')}`)
  }
  const named = positionals.length === 0 ? ['backend', 'web'] : positionals
  const wanted = SERVERS.filter((server) => named.includes(server) || (server === 'backend' && named.includes('web')))
  const gallery = wanted.includes('gallery') ? startGallery(context) : Promise.resolve()
  if (wanted.includes('backend')) {
    await startBackend(context)
  }
  if (wanted.includes('web')) {
    await startWeb(context)
  }
  await gallery
}

/** Prints that a server already runs, when it does; answers whether it did. */
function alreadyUp(context: Context, name: ServerName, title: string, port: number): boolean {
  if (!runningServer(context.slot, name)) {
    return false
  }
  context.print(`${title.padEnd(9)} ${local(port)}  already up`)
  return true
}

/** Starts `command` as the slot's server `name` and records its group. */
async function start(context: Context, name: ServerName, port: number, command: string[], extra: Record<string, string>): Promise<Started> {
  if (await listening(port)) {
    throw new CommandFailure(`Port ${port}, the slot's ${name} port, is taken by a program this slot did not start. Stop it, or find it with lsof -i :${port}.`)
  }
  const started = startGroup(command, { cwd: context.slot.root, env: childEnv(context, extra), log: freshLog(context.slot, name) })
  updateState(context.slot, (state) => {
    state.servers[name] = started.group
  })
  return started
}

async function startBackend(context: Context): Promise<void> {
  const port = context.slot.ports.backend
  if (alreadyUp(context, 'backend', 'Backend', port)) {
    return
  }
  borrowEnv(context)
  const began = Date.now()
  const command = [process.execPath, 'scripts/xtask.ts', 'dev', '--port', String(port), '--data', slotPaths(context.slot).backendData]
  const started = await start(context, 'backend', port, command, {})
  const account = await untilReady('The backend', started, async () => {
    if (!lineOf(started.group.log, BACKEND_READY)) {
      return null
    }
    return lineOf(started.group.log, ACCOUNT)
  }, BACKEND_START_MS)
  const email = account[1]
  const password = account[2] === 'from DEMI_DEV_PASSWORD' ? null : account[2]
  updateState(context.slot, (state) => {
    state.account = { email, password }
  })
  context.print(`Backend   ${local(port)}  ready in ${seconds(began)}`)
}

/**
 * Copies the user's `.env` into a slot that has none, for the backend's
 * development model and account; `down` deletes it again.
 */
function borrowEnv(context: Context): void {
  const checkout = userCheckout(context.slot.root)
  const target = join(context.slot.root, '.env')
  if (checkout === null || existsSync(target) || !existsSync(join(checkout, '.env'))) {
    return
  }
  copyFileSync(join(checkout, '.env'), target)
  updateState(context.slot, (state) => {
    state.envCopied = true
  })
  context.print(`Copied ${join(checkout, '.env')} into the slot; down deletes it`)
}

async function startWeb(context: Context): Promise<void> {
  const { ports } = context.slot
  const account = readState(context.slot).account
  if (!account) {
    throw new CommandFailure('The slot\'s state has no development account; run bun browse down, then bun browse up')
  }
  if (!alreadyUp(context, 'web', 'Web', ports.web)) {
    const began = Date.now()
    // The sign-in page fills in the account `.env` names; any other is the
    // backend's fixed one, which the page learns from the same variables.
    const extra: Record<string, string> = { DEMI_BACKEND_URL: local(ports.backend) }
    if (account.password !== null) {
      extra.DEMI_DEV_EMAIL = account.email
      extra.DEMI_DEV_PASSWORD = account.password
    }
    const started = await start(context, 'web', ports.web, [process.execPath, 'scripts/web-dev.ts', '--port', String(ports.web), '--strictPort'], extra)
    await untilReady('The web app', started, async () => (await answersOk(local(ports.web)) ? true : null), VITE_START_MS)
    context.print(`Web       ${local(ports.web)}  ready in ${seconds(began)}`)
  }
  await signIn(context, account)
}

/** Signs the browser in through the web app, as its sign-in form does. */
async function signIn(context: Context, account: { email: string, password: string | null }): Promise<void> {
  const password = account.password ?? dotEnvValue(context.slot.root, 'DEMI_DEV_PASSWORD')
  if (password === undefined) {
    throw new CommandFailure('The backend took the account\'s password from DEMI_DEV_PASSWORD, which the slot\'s .env no longer has')
  }
  await context.network()
  const page = await context.browser.page()
  const base = webBase(context.slot)
  const answer = await page.request.post(`${base}/api/auth/login`, { data: { email: account.email, password } })
  if (!answer.ok()) {
    throw new CommandFailure(`Signing in as ${account.email} failed: ${answer.status()} ${await answer.text()}`)
  }
  await page.goto(`${base}/`)
  // The app sends a signed-in page from `/` to its conversations, and
  // any other to the sign-in page.
  await page.waitForURL((url) => url.pathname.startsWith('/chat') || url.pathname.startsWith('/login'), { timeout: 30_000 })
  if (!new URL(page.url()).pathname.startsWith('/chat')) {
    throw new CommandFailure(`The backend signed ${account.email} in, but the page shows ${page.url()}`)
  }
  context.print(`          signed in as ${account.email}; the browser reaches the web app at ${base}, where net acts`)
}

async function startGallery(context: Context): Promise<void> {
  const port = context.slot.ports.gallery
  if (alreadyUp(context, 'gallery', 'Gallery', port)) {
    return
  }
  const began = Date.now()
  const started = await start(context, 'gallery', port, [process.execPath, 'scripts/web-gallery.ts', '--port', String(port), '--strictPort'], {})
  await untilReady('The gallery', started, async () => (await answersOk(local(port)) ? true : null), VITE_START_MS)
  context.print(`Gallery   ${local(port)}  ready in ${seconds(began)}`)
}

function seconds(since: number): string {
  return `${Math.round((Date.now() - since) / 1000)} s`
}

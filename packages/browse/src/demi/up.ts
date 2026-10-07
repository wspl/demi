// `demi.up(...servers)` (browse.md § What `demi` adds): starts what it names
// on the slot's ports, the backend and the web app when it names nothing,
// waits until each answers, and signs the browser in with the development
// account once the web app is up, unless its session is still good, keeping
// the page it shows. The web app needs the backend, so naming it starts the
// backend too. Each server leads a process group the slot's state records
// for `demi.down`. The backend keeps its data in the slot's folder, so it
// comes back with the account and the conversations it had until
// `demi.down({ wipe: true })`.
import { copyFileSync, existsSync } from 'node:fs'
import { join } from 'node:path'
import { identitySchema } from '@demicodes/web/src/api/generated/web-api'
import { errors, type Page } from 'playwright'
import { startGroup, type Started } from '../processes'
import {
  answersOk, childEnv, dotEnvValue, freshLog, lineOf, listening, runningServer, untilReady, userCheckout,
} from '../servers'
import { local, slotPaths } from '../slot'
import { readState, updateState, type ServerName } from '../state'
import { Failure, webBase, type Tool } from '../tool'

export const SERVERS = ['backend', 'web', 'gallery'] as const

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

export async function up(tool: Tool, named: string[]): Promise<void> {
  const unknown = named.filter((name) => !SERVERS.some((server) => server === name))
  if (unknown.length > 0) {
    throw new Failure(`demi.up starts ${SERVERS.join(', ')}, not ${unknown.join(', ')}`)
  }
  const asked = named.length === 0 ? ['backend', 'web'] : named
  const wanted = SERVERS.filter((server) => asked.includes(server) || (server === 'backend' && asked.includes('web')))
  const gallery = wanted.includes('gallery') ? startGallery(tool) : Promise.resolve()
  const app = (async () => {
    if (wanted.includes('backend')) {
      await startBackend(tool)
    }
    if (wanted.includes('web')) {
      await startWeb(tool)
    }
  })()
  // The gallery starts meanwhile; the first failure is thrown, and neither goes unhandled.
  for (const outcome of await Promise.allSettled([app, gallery])) {
    if (outcome.status === 'rejected') {
      throw outcome.reason
    }
  }
}

/** Starts `command` as the slot's server `name` and records its group. */
async function start(tool: Tool, name: ServerName, port: number, command: string[], extra: Record<string, string>): Promise<Started> {
  if (await listening(port)) {
    throw new Failure(`Port ${port}, the slot's ${name} port, is taken by a program this slot did not start. Stop it, or find it with lsof -i :${port}.`)
  }
  const started = startGroup(command, { cwd: tool.slot.root, env: childEnv(tool, extra), log: freshLog(tool.slot, name) })
  updateState(tool.slot, (state) => {
    state.servers[name] = started.group
  })
  return started
}

/** The line a server's start prints: its title, its address, and what happened. */
function report(tool: Tool, title: string, address: string, what: string): void {
  tool.print(`${title.padEnd(9)} ${address}  ${what}`)
}

async function startBackend(tool: Tool): Promise<void> {
  const port = tool.slot.ports.backend
  if (runningServer(tool.slot, 'backend')) {
    report(tool, 'Backend', local(port), 'already up')
    return
  }
  borrowEnv(tool)
  const began = Date.now()
  // The preview namespace admits the web app's origin and the slot's
  // network's, which the browser loads it through.
  const command = [
    process.execPath, 'scripts/xtask.ts', 'dev', '--port', String(port), '--data', slotPaths(tool.slot).backendData,
    '--preview-port', String(tool.slot.ports.preview),
    '--web-origin', local(tool.slot.ports.web), '--web-origin', webBase(tool.slot),
  ]
  const started = await start(tool, 'backend', port, command, {})
  const account = await untilReady('The backend', started, async () => {
    if (!lineOf(started.group.log, BACKEND_READY)) {
      return null
    }
    return lineOf(started.group.log, ACCOUNT)
  }, BACKEND_START_MS)
  const email = account[1]
  const password = account[2] === 'from DEMI_DEV_PASSWORD' ? null : account[2]
  updateState(tool.slot, (state) => {
    state.account = { email, password }
  })
  report(tool, 'Backend', local(port), `ready in ${seconds(began)}`)
}

/**
 * Copies the user's `.env` into a slot that has none, for the backend's
 * development model and account; `demi.down` deletes it again.
 */
function borrowEnv(tool: Tool): void {
  const checkout = userCheckout(tool.slot.root)
  const target = join(tool.slot.root, '.env')
  if (checkout === null || existsSync(target) || !existsSync(join(checkout, '.env'))) {
    return
  }
  copyFileSync(join(checkout, '.env'), target)
  updateState(tool.slot, (state) => {
    state.envCopied = true
  })
  tool.print(`Copied ${join(checkout, '.env')} into the slot; demi.down deletes it`)
}

async function startWeb(tool: Tool): Promise<void> {
  const { ports } = tool.slot
  const account = readState(tool.slot).account
  if (!account) {
    throw new Failure('The slot\'s state has no development account; run bun browse down, then bun browse up')
  }
  let started = 'already up'
  if (!runningServer(tool.slot, 'web')) {
    const began = Date.now()
    // The sign-in page fills in the account `.env` names; any other is the
    // backend's fixed one, which the page learns from the same variables.
    const extra: Record<string, string> = { DEMI_BACKEND_URL: local(ports.backend) }
    if (account.password !== null) {
      extra.DEMI_DEV_EMAIL = account.email
      extra.DEMI_DEV_PASSWORD = account.password
    }
    const server = await start(tool, 'web', ports.web, [process.execPath, 'scripts/web-dev.ts', '--port', String(ports.web), '--strictPort'], extra)
    await untilReady('The web app', server, async () => (await answersOk(local(ports.web)) ? true : null), VITE_START_MS)
    started = `ready in ${seconds(began)}`
  }
  const signedIn = await signIn(tool, account)
  // The browser reaches the web app through the slot's network, where `demi.net` acts.
  report(tool, 'Web', webBase(tool.slot), `${started}, ${signedIn}`)
}

/** Pages of the web app a signed-in page never stays on: the sign-in and setup pages, and `/`, which goes on to the conversations. */
function signedOutPage(path: string): boolean {
  return path === '/' || path.startsWith('/login') || path.startsWith('/setup')
}

/**
 * Signs the browser in with the development account when the backend does
 * not know its session, as the sign-in form does, and keeps the page it
 * shows: a page of the web app stays, the sign-in page goes on to the page
 * it was opened for, and a page elsewhere opens the web app. Answers what
 * it did, such as `signed in as developer@example.test`.
 */
async function signIn(tool: Tool, account: { email: string, password: string | null }): Promise<string> {
  await tool.network()
  const page = await tool.browser.page()
  const base = webBase(tool.slot)
  const shown = new URL(page.url())
  const session = await page.request.get(`${base}/api/auth/me`)
  let email: string
  let signedInNow: boolean
  if (session.ok()) {
    email = identitySchema.parse(await session.json()).user.email
    signedInNow = false
  } else if (session.status() === 401) {
    email = await logIn(tool, page, account)
    signedInNow = true
  } else {
    throw new Failure(`Asking the backend for the browser's session failed: ${session.status()} ${await session.text()}`)
  }
  if (shown.origin !== new URL(base).origin) {
    await page.goto(`${base}/`)
  } else if (signedInNow || signedOutPage(shown.pathname)) {
    // The app reads the session as it loads: the page loads again, and a
    // sign-in page goes on to the page it was opened for.
    await page.goto(shown.href)
  }
  try {
    await page.waitForURL((url) => !signedOutPage(url.pathname), { timeout: 15_000 })
  } catch (error) {
    if (!(error instanceof errors.TimeoutError)) {
      throw error
    }
    throw new Failure(`The backend signed ${email} in, but the page stays at ${page.url()}`)
  }
  return `${signedInNow ? 'signed in as' : 'already signed in as'} ${email}`
}

/** Signs the browser's context in with `account`, as the sign-in form does; answers the email signed in. */
async function logIn(tool: Tool, page: Page, account: { email: string, password: string | null }): Promise<string> {
  const password = account.password ?? dotEnvValue(tool.slot.root, 'DEMI_DEV_PASSWORD')
  if (password === undefined) {
    throw new Failure('The backend took the account\'s password from DEMI_DEV_PASSWORD, which the slot\'s .env no longer has')
  }
  const answer = await page.request.post(`${webBase(tool.slot)}/api/auth/login`, { data: { email: account.email, password } })
  if (!answer.ok()) {
    throw new Failure(`Signing in as ${account.email} failed: ${answer.status()} ${await answer.text()}`)
  }
  return identitySchema.parse(await answer.json()).user.email
}

async function startGallery(tool: Tool): Promise<void> {
  const port = tool.slot.ports.gallery
  if (runningServer(tool.slot, 'gallery')) {
    report(tool, 'Gallery', local(port), 'already up')
    return
  }
  const began = Date.now()
  const started = await start(tool, 'gallery', port, [process.execPath, 'scripts/web-gallery.ts', '--port', String(port), '--strictPort'], {})
  await untilReady('The gallery', started, async () => (await answersOk(local(port)) ? true : null), VITE_START_MS)
  report(tool, 'Gallery', local(port), `ready in ${seconds(began)}`)
}

function seconds(since: number): string {
  return `${Math.round((Date.now() - since) / 1000)} s`
}

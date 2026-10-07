// The slot's browser (product-checks.md § One browser per slot): Playwright's
// Chromium with its own profile in the slot's folder, started by the first
// command that needs it and kept between commands by the daemon. A browser
// that crashed or was closed starts again on the next command.
import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { chromium, devices, type BrowserContext, type CDPSession, type Page } from 'playwright'
import { Logs } from './logs'
import { slotPaths, type Slot } from './slot'
import { readState, updateState, type Emulation } from './state'

/** The viewport and pixel ratio of a browser no `emulate` changed. */
const DEFAULT_VIEWPORT = { width: 1440, height: 900 }
const DEFAULT_SCALE = 2

export class Browser {
  private launched: Promise<BrowserContext> | null = null
  /** The context `launched` gave, once it has started. */
  private open: BrowserContext | null = null
  private current: Page | null = null
  private readonly sessions = new WeakMap<Page, CDPSession>()
  readonly logs = new Logs()
  /** Whether the browser shows its window. */
  private headed = false
  /** Whether a command asked for the window, which the browser keeps until the daemon ends. */
  private wantsWindow = false

  constructor(
    private readonly slot: Slot,
    private readonly print: (line: string) => void,
  ) {}

  /** Whether a browser runs now. */
  get running(): boolean {
    return this.launched !== null
  }

  /** Asks for the browser's window (`--headed`): a headless browser starts again with one. */
  showWindow(): void {
    this.wantsWindow = true
  }

  /** The browser's context, started when none runs. */
  async context(): Promise<BrowserContext> {
    if (this.launched && this.wantsWindow && !this.headed) {
      await this.relaunch()
    }
    if (!this.launched) {
      this.headed = this.wantsWindow
      this.launched = this.launch()
      // A failed start leaves nothing for the next command to reuse.
      this.launched.catch(() => {
        this.launched = null
      })
    }
    return this.launched
  }

  private async launch(): Promise<BrowserContext> {
    if (!existsSync(chromium.executablePath())) {
      await install(this.print)
    }
    const emulation = readState(this.slot).emulation ?? {}
    const context = await chromium.launchPersistentContext(slotPaths(this.slot).profile, {
      headless: !this.headed,
      ...launchOptions(emulation),
    })
    this.logs.reset()
    this.logs.watch(context)
    this.open = context
    // A browser that crashed or was closed by hand starts again on the next
    // command; one that `close` ended has been forgotten already.
    context.on('close', () => {
      if (this.open === context) {
        this.open = null
        this.launched = null
        this.current = null
      }
    })
    return context
  }

  /** The page commands act on: the one last opened, or the browser's first. */
  async page(): Promise<Page> {
    const context = await this.context()
    if (this.current && !this.current.isClosed()) {
      return this.current
    }
    const page = context.pages().find((open) => !open.isClosed()) ?? await context.newPage()
    this.current = page
    const reopen = readState(this.slot).reopen
    if (reopen !== undefined) {
      updateState(this.slot, (state) => {
        delete state.reopen
      })
      await page.goto(reopen)
    }
    return page
  }

  /** Keeps the page's address for the browser of the next daemon. */
  keepAddress(): void {
    const address = this.current && !this.current.isClosed() ? this.current.url() : undefined
    if (address && address !== 'about:blank') {
      updateState(this.slot, (state) => {
        state.reopen = address
      })
    }
  }

  /** A DevTools protocol session with the page. */
  async cdp(): Promise<CDPSession> {
    const page = await this.page()
    const known = this.sessions.get(page)
    if (known) {
      return known
    }
    const session = await page.context().newCDPSession(page)
    this.sessions.set(page, session)
    return session
  }

  /** The page when a browser runs, without starting one. */
  async existingPage(): Promise<Page | null> {
    return this.launched ? this.page() : null
  }

  /**
   * Starts the browser again with the emulation the state now holds, and
   * reopens the address the page showed.
   */
  async relaunch(): Promise<void> {
    const page = this.current
    const address = page && !page.isClosed() ? page.url() : undefined
    await this.close()
    const reopened = await this.page()
    if (address && address !== 'about:blank') {
      await reopened.goto(address)
    }
  }

  async close(): Promise<void> {
    const launched = this.launched
    this.launched = null
    this.open = null
    this.current = null
    if (launched) {
      // A browser that failed to start has nothing to close.
      await launched.then((context) => context.close(), () => undefined)
    }
  }
}

/** The context options `emulation` asks for, a named device's first. */
export function launchOptions(emulation: Emulation) {
  const device = emulation.device === undefined ? undefined : devices[emulation.device]
  if (emulation.device !== undefined && !device) {
    throw new Error(`Playwright knows no device ${emulation.device}`)
  }
  const viewport = {
    width: emulation.width ?? device?.viewport.width ?? DEFAULT_VIEWPORT.width,
    height: emulation.height ?? device?.viewport.height ?? DEFAULT_VIEWPORT.height,
  }
  return {
    ...device,
    viewport,
    deviceScaleFactor: emulation.scale ?? device?.deviceScaleFactor ?? DEFAULT_SCALE,
    colorScheme: emulation.theme,
    locale: emulation.locale,
    timezoneId: emulation.timezone,
  }
}

/** Installs Playwright's Chromium with Playwright's own installer, into its usual cache. */
async function install(print: (line: string) => void): Promise<void> {
  print('Installing Playwright\'s Chromium, once for every checkout')
  const require = createRequire(import.meta.url)
  const cli = join(dirname(require.resolve('playwright/package.json')), 'cli.js')
  const installer = Bun.spawn([process.execPath, cli, 'install', 'chromium'], { stdout: 'pipe', stderr: 'pipe' })
  const [code, output, errors] = await Promise.all([
    installer.exited,
    new Response(installer.stdout).text(),
    new Response(installer.stderr).text(),
  ])
  if (code !== 0) {
    throw new Error(`Playwright's installer failed:\n${output}${errors}`)
  }
}

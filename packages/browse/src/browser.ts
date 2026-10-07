// The slot's browser (browse.md § One browser per slot): Playwright's
// Chromium with its own profile in the slot's folder, started by the first
// command that needs it as a process group of its own, which the slot's
// state records with its DevTools endpoint. The daemon reaches it over the
// DevTools protocol, so a daemon that ends for changed code leaves the
// browser, its pages and what was typed in them, and the next daemon
// attaches to it again. A browser that crashed or was closed starts again on
// the next command; `stop`, `down` and the idle timeout stop it.
import { existsSync, readFileSync, rmSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { chromium, type BrowserContext, type CDPSession, type Browser as PlaywrightBrowser, type Page } from 'playwright'
import { CommandFailure } from './command'
import { applyEmulation } from './emulation'
import { Logs } from './logs'
import { ownGroupRuns, startGroup, stopGroup } from './processes'
import { freshLog, untilReady } from './servers'
import { local, slotPaths, type Slot } from './slot'
import { readState, updateState, type Emulation } from './state'
import type { Clip } from './shots'

/** How long the browser may take to open its DevTools endpoint. */
const START_MS = 30_000
/** How long the browser may take to stop before it is killed. */
const STOP_MS = 10_000
/** How long attaching to a browser only to ask it to quit may take. */
const QUIT_MS = 5_000

/**
 * The switches the browser starts with: Playwright's own for its Chromium
 * (its `chromiumSwitches`, which it does not export), so the browser and
 * its pictures are those a check with Playwright's launch had: a page in the
 * background or a hidden window keeps its timers and paints, no first-run
 * or update prompt appears, and colours are those of a screenshot.
 */
const SWITCHES = [
  '--disable-field-trial-config',
  '--disable-background-networking',
  '--disable-background-timer-throttling',
  '--disable-backgrounding-occluded-windows',
  '--disable-back-forward-cache',
  '--disable-breakpad',
  '--disable-client-side-phishing-detection',
  '--disable-component-extensions-with-background-pages',
  '--disable-component-update',
  '--no-default-browser-check',
  '--disable-default-apps',
  '--disable-dev-shm-usage',
  '--disable-extensions',
  '--disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding,ThirdPartyStoragePartitioning,Translate,OptimizationHints',
  '--allow-pre-commit-input',
  '--disable-hang-monitor',
  '--disable-ipc-flooding-protection',
  '--disable-popup-blocking',
  '--disable-prompt-on-repost',
  '--disable-renderer-backgrounding',
  '--force-color-profile=srgb',
  '--metrics-recording-only',
  '--no-first-run',
  '--password-store=basic',
  '--use-mock-keychain',
  '--no-service-autorun',
  '--disable-search-engine-choice-screen',
  '--disable-infobars',
  '--disable-sync',
  '--window-size=1440,900',
]

/** What Playwright adds for a headless browser, so hover and pointer media queries answer as they did. */
const HEADLESS = [
  '--headless',
  '--hide-scrollbars',
  '--mute-audio',
  '--blink-settings=primaryHoverType=2,availableHoverTypes=2,primaryPointerType=4,availablePointerTypes=4',
]

/** The daemon's connection to the browser. */
interface Connection {
  browser: PlaywrightBrowser
  context: BrowserContext
  headed: boolean
  /** The browser's own user agent, which a page that stops emulating a phone gets back. */
  userAgent: string
}

export class Browser {
  private connected: Promise<Connection> | null = null
  private current: Page | null = null
  /** The daemon's session with each page, which holds the page's emulation. */
  private readonly sessions = new WeakMap<Page, Promise<CDPSession>>()
  readonly logs = new Logs()
  /** Whether a command asked for the window, which the browser keeps until it stops. */
  private wantsWindow = false

  constructor(
    private readonly slot: Slot,
    private readonly print: (line: string) => void,
  ) {}

  /** Whether the daemon is attached to a browser now. */
  get running(): boolean {
    return this.connected !== null
  }

  /** Asks for the browser's window (`--headed`): a headless browser starts again with one. */
  showWindow(): void {
    this.wantsWindow = true
  }

  /** The browser's context, its browser started or attached to when the daemon has none. */
  async context(): Promise<BrowserContext> {
    const connection = await this.connection()
    if (!this.wantsWindow || connection.headed) {
      return connection.context
    }
    // A window is asked for: the browser starts again with one, on the address the page showed.
    const page = this.current
    const address = page && !page.isClosed() ? page.url() : undefined
    await this.close()
    const reopened = await this.page()
    if (address && address !== 'about:blank') {
      await reopened.goto(address)
    }
    return reopened.context()
  }

  private connection(): Promise<Connection> {
    if (!this.connected) {
      const connecting = this.connect()
      this.connected = connecting
      // A failed start leaves nothing for the next command to reuse.
      connecting.catch(() => {
        if (this.connected === connecting) {
          this.connected = null
        }
      })
    }
    return this.connected
  }

  /** Attaches to the slot's browser, started first when none runs or it has no window one is asked for. */
  private async connect(): Promise<Connection> {
    const recorded = readState(this.slot).browser
    const runs = recorded !== undefined && ownGroupRuns(recorded)
    let endpoint: string
    let headed: boolean
    if (runs && (recorded.headed || !this.wantsWindow)) {
      endpoint = recorded.endpoint
      headed = recorded.headed
    } else {
      if (runs) {
        await stopGroup(recorded, STOP_MS)
      }
      headed = this.wantsWindow
      endpoint = await this.launch(headed)
    }
    const browser = await chromium.connectOverCDP(endpoint)
    const context = browser.contexts()[0]
    if (!context) {
      throw new CommandFailure(`The browser at ${endpoint} has no default context`)
    }
    const version = await (await browser.newBrowserCDPSession()).send('Browser.getVersion')
    const connection: Connection = { browser, context, headed, userAgent: version.userAgent }
    browser.on('disconnected', () => {
      // A browser that crashed or was closed by hand starts again on the next command.
      if (this.current?.context() === context) {
        this.current = null
      }
      void this.connected?.then((open) => {
        if (open === connection) {
          this.connected = null
        }
      })
    })
    this.logs.reset()
    this.logs.watch(context)
    context.on('page', (page) => {
      // A page that closes as it opens, such as a popup, needs no emulation.
      this.session(page, connection).catch(() => undefined)
    })
    for (const page of context.pages()) {
      await this.session(page, connection)
    }
    return connection
  }

  /** Starts the browser process; answers its DevTools endpoint. */
  private async launch(headed: boolean): Promise<string> {
    if (!existsSync(chromium.executablePath())) {
      await install(this.print)
    }
    const paths = slotPaths(this.slot)
    const portFile = join(paths.profile, 'DevToolsActivePort')
    // The file of a browser that ran before names a port nobody listens on.
    rmSync(portFile, { force: true })
    // A browser that was killed or crashed left its tabs to restore; a new
    // browser opens on a blank page instead, since a restored tab that
    // never loads would keep the daemon from attaching.
    rmSync(join(paths.profile, 'Default', 'Sessions'), { recursive: true, force: true })
    const command = [
      chromium.executablePath(),
      `--user-data-dir=${paths.profile}`,
      '--remote-debugging-port=0',
      ...SWITCHES,
      ...headed ? [] : HEADLESS,
      'about:blank',
    ]
    const started = startGroup(command, { cwd: this.slot.root, env: process.env, log: freshLog(this.slot, 'browser') })
    let port: string
    try {
      port = await untilReady('The browser', started, async () => {
        if (!existsSync(portFile)) {
          return null
        }
        // The browser writes the port on the first line once it listens.
        const first = readFileSync(portFile, 'utf8').split('\n')[0]
        return /^\d+$/.test(first) ? first : null
      }, START_MS)
    } catch (error) {
      // The state records no browser that never listened, so nothing else would stop it.
      await stopGroup(started.group, STOP_MS)
      throw error
    }
    const endpoint = local(Number(port))
    updateState(this.slot, (state) => {
      state.browser = { ...started.group, endpoint, headed }
    })
    return endpoint
  }

  /** The daemon's session with `page`, opened with the slot's emulation applied the first time. */
  private session(page: Page, connection: Connection): Promise<CDPSession> {
    const known = this.sessions.get(page)
    if (known) {
      return known
    }
    const opening = (async () => {
      const session = await connection.context.newCDPSession(page)
      await applyEmulation(session, readState(this.slot).emulation ?? {}, connection.userAgent)
      return session
    })()
    this.sessions.set(page, opening)
    opening.catch(() => this.sessions.delete(page))
    return opening
  }

  /** The page commands act on: the one last used, or the browser's first. */
  async page(): Promise<Page> {
    const context = await this.context()
    if (this.current && !this.current.isClosed()) {
      return this.current
    }
    const page = context.pages().find((open) => !open.isClosed()) ?? await context.newPage()
    this.current = page
    return page
  }

  /** The daemon's DevTools protocol session with the page, which holds its emulation. */
  async cdp(): Promise<CDPSession> {
    const page = await this.page()
    return this.session(page, await this.connection())
  }

  /** The page when the daemon is attached to a browser, without starting or attaching to one. */
  async existingPage(): Promise<Page | null> {
    return this.connected ? this.page() : null
  }

  /** Applies `emulation` to every page of an attached browser; a browser attached later gets it then. */
  async emulate(emulation: Emulation): Promise<void> {
    if (!this.connected) {
      return
    }
    const connection = await this.connected
    for (const page of connection.context.pages()) {
      await applyEmulation(await this.session(page, connection), emulation, connection.userAgent)
    }
  }

  /**
   * A PNG of the page through the daemon's own session, whose emulation
   * survives the capture: one taken through another session, as
   * Playwright's screenshots are, drops the pixel ratio the page emulates.
   * `clip` is in CSS pixels of the viewport; `scale` multiplies the page's
   * pixel ratio; `full` takes the whole page.
   */
  async capture(options: { clip?: Clip, scale?: number, full?: boolean } = {}): Promise<Buffer> {
    const session = await this.cdp()
    const metrics = await session.send('Page.getLayoutMetrics')
    const viewport = metrics.cssVisualViewport
    let clip: Clip
    if (options.full) {
      clip = { x: 0, y: 0, width: metrics.cssContentSize.width, height: metrics.cssContentSize.height }
    } else {
      // The protocol's clip is in the page's coordinates, the viewport's moved by its scroll.
      const area = options.clip ?? { x: 0, y: 0, width: viewport.clientWidth, height: viewport.clientHeight }
      clip = { ...area, x: area.x + viewport.pageX, y: area.y + viewport.pageY }
    }
    const { data } = await session.send('Page.captureScreenshot', {
      format: 'png',
      clip: { ...clip, scale: options.scale ?? 1 },
      captureBeyondViewport: options.full ?? false,
    })
    return Buffer.from(data, 'base64')
  }

  /**
   * Quits the browser and forgets it; the next command starts a new one.
   * It is asked to quit as a person quits it, which a signal is not on
   * macOS: a browser ended by one restores its tabs when it starts again.
   */
  async close(): Promise<void> {
    const connected = this.connected
    this.connected = null
    this.current = null
    const recorded = readState(this.slot).browser
    if (!recorded) {
      return
    }
    if (ownGroupRuns(recorded)) {
      try {
        const connection = connected ? await connected : null
        const browser = connection?.browser ?? await chromium.connectOverCDP(recorded.endpoint, { timeout: QUIT_MS })
        await (await browser.newBrowserCDPSession()).send('Browser.close')
      } catch (error) {
        // A browser that does not answer is stopped by its group below.
        this.print(`The browser did not quit when asked (${error instanceof Error ? error.message.split('\n')[0] : error}); stopping it`)
      }
      await stopGroup(recorded, STOP_MS)
    }
    updateState(this.slot, (state) => {
      delete state.browser
    })
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

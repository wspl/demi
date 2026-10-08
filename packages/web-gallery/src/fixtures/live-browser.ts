/**
 * A live browser view without a Host (`live-view.md`): the gallery
 * draws a page, encodes it as the Host's capture would, and speaks the live
 * protocol, so the view's pictures, input, controls and dialogs show here.
 */
import { LIVE_CAPTURE_STOPPED, LIVE_CAPTURE_UNAVAILABLE, LIVE_CONTROL_FRAME, LIVE_VIDEO_CODEC } from '@demicodes/plugin-browser/generated/plugin'
import type {
  BrowserTab,
  BrowserViewport,
  CursorRegion,
  LiveControl,
  LiveDownload,
  LiveModuleMessage,
  LiveTab,
  LiveViewerMessage,
} from '@demicodes/plugin-browser/generated/plugin'
import { shallowRef, type ShallowRef } from 'vue'
import { encodeVideo } from '@demicodes/plugin-browser/live/frames'
import type { OpenUserStream, UserStreamHandlers } from '@demicodes/web-ui/plugins/streams'
import { CONTROL, META } from '@demicodes/plugin-browser/live/input'
import { BrowserTabsError, type BrowserTabList, type StartingPhase } from '@demicodes/plugin-browser/live/tabs'
import type { HostArtifact } from '@demicodes/web-ui/devices/installed'
import { WORKSPACE_ROOT } from './workspace'

const FPS = 10
const encoder = new TextEncoder()
const decoder = new TextDecoder()

function framed(kind: number, payload: Uint8Array): Uint8Array {
  const bytes = new Uint8Array(5 + payload.length)
  new DataView(bytes.buffer).setUint32(0, 1 + payload.length)
  bytes[4] = kind
  bytes.set(payload, 5)
  return bytes
}

function message(value: LiveModuleMessage): Uint8Array {
  return framed(LIVE_CONTROL_FRAME, encoder.encode(JSON.stringify(value)))
}

function picture(
  tab: string,
  generation: number,
  sequence: number,
  chunk: EncodedVideoChunk,
  size: { width: number; height: number },
): Uint8Array {
  const data = new Uint8Array(chunk.byteLength)
  chunk.copyTo(data)
  return encodeVideo({
    tab,
    generation,
    sequence,
    key: chunk.type === 'key',
    timestamp: chunk.timestamp,
    width: size.width,
    height: size.height,
    data,
  })
}

const SELECT: LiveControl = {
  token: '2d1f7a0c-6f2b-4d5e-9d6d-2b4f7a0c6f2b',
  revision: 0,
  kind: 'select',
  label: 'Status',
  value: 'open',
  min: '', max: '', step: '', accept: '',
  multiple: false, disabled: false, required: false, size: 0,
  options: [
    { label: 'Open', value: 'open', group: '', disabled: false, hidden: false, selected: true },
    { label: 'Shipped', value: 'shipped', group: '', disabled: false, hidden: false, selected: false },
  ],
  rect: { x: 24, y: 168, width: 180, height: 32 },
}

/** The drawn page's link that opens the invoice in a new tab, as a `target=_blank` link does. */
const INVOICE = { x: 184, y: 98, width: 200, height: 18, url: 'https://example.test/invoices/4711' }

/** The titles the drawn pages give themselves once loaded, as a page's `<title>` does; another keeps its host. */
const PAGE_TITLES: ReadonlyMap<string, string> = new Map([[INVOICE.url, 'Invoice 4711 — Example']])
/** The drawn page's link that downloads a file, which the gallery's workspace holds where the Host would save it. */
const DOWNLOAD = { x: 184, y: 118, width: 140, height: 18, url: 'https://example.test/guide.pdf' }
const DOWNLOADED: Pick<LiveDownload, 'name' | 'total' | 'path'> = {
  name: 'guide.pdf',
  total: 734,
  path: `${WORKSPACE_ROOT}/docs/guide.pdf`,
}
/** How long the drawn page's download takes, so its progress shows in the bubble. */
const DOWNLOAD_MS = 1500

function within(area: { x: number; y: number; width: number; height: number }, x: number, y: number): boolean {
  return x >= area.x && x <= area.x + area.width && y >= area.y && y <= area.y + area.height
}

/** Where the drawn page shows which cursor, as a page's observer reports it: the button, the select, the field and the links. */
function pageCursors(viewport: BrowserViewport): CursorRegion[] {
  return [
    { x: 24, y: 96, width: 136, height: 40, cursor: 'pointer' },
    { x: 24, y: 168, width: 180, height: 32, cursor: 'default' },
    { x: 24, y: 216, width: viewport.width - 48, height: 36, cursor: 'text' },
    { x: INVOICE.x, y: INVOICE.y, width: INVOICE.width, height: INVOICE.height, cursor: 'pointer' },
    { x: DOWNLOAD.x, y: DOWNLOAD.y, width: DOWNLOAD.width, height: DOWNLOAD.height, cursor: 'pointer' },
  ]
}

/** Where the drawn page's text stands, which the browser's own cursor resolves to a text cursor over. */
function overText(x: number, y: number): boolean {
  return (x >= 24 && x <= 110 && y >= 28 && y <= 54) || (x >= 24 && x <= 290 && y >= 62 && y <= 82)
}

/** H.264 encodes only even sides: an odd one loses its last row or column, as the Host's capture cuts it. */
function even(length: number): number {
  return Math.max(2, length - (length % 2))
}

/** How long the gallery's conversation browser takes over a request, as a Host takes a moment. */
const REQUEST_DELAY_MS = 900
/**
 * How long after its answer the browser starts loading the page a request
 * asked for, as a far Host's tab list reaches the page after its answer.
 */
const LOAD_START_MS = 700
/** How long a page the gallery's browser loads takes, as a slow page on a far Host does: long enough to stop it. */
const LOAD_MS = 5000
/** How long a stopped Cloud takes to start, as a Cloud wakes. */
const CLOUD_START_MS = 2500
/** How long the browser takes to start on a Host where it does not run, as Chrome starts. */
const BROWSER_START_MS = 1500
/** Each site's icon, drawn once. */
const siteIcons = new Map<string, string>()

/**
 * A site's icon as the Host draws it, 32 pixels square: the first letter of
 * its host on a color of its own. `plain.test` has none, so its tabs show
 * the generic mark, as a site without an icon does.
 */
function siteIcon(url: string): string | undefined {
  const parsed = URL.parse(url)
  if (!parsed || (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') || parsed.host === 'plain.test') {
    return undefined
  }
  const known = siteIcons.get(parsed.host)
  if (known) {
    return known
  }
  const canvas = document.createElement('canvas')
  canvas.width = 32
  canvas.height = 32
  const context = canvas.getContext('2d')
  if (!context) {
    return undefined
  }
  const hue = [...parsed.host].reduce((sum, letter) => sum + letter.charCodeAt(0), 0) % 360
  context.fillStyle = `hsl(${hue} 60% 45%)`
  context.beginPath()
  context.roundRect(0, 0, 32, 32, 7)
  context.fill()
  context.fillStyle = '#fff'
  context.font = 'bold 20px sans-serif'
  context.textAlign = 'center'
  context.textBaseline = 'middle'
  context.fillText(parsed.host.charAt(0).toUpperCase(), 16, 17)
  const icon = canvas.toDataURL('image/png')
  siteIcons.set(parsed.host, icon)
  return icon
}

/** Tab ids as the protocol spells them: `t` and the tab's number in the conversation. */
function galleryTabs(): LiveTab[] {
  return [
    {
      id: 't1',
      title: 'Orders — Example',
      url: 'https://example.test/orders',
      createdBy: { kind: 'agent', number: 0 },
      viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
      loading: false,
      canGoBack: false,
      canGoForward: false,
      favicon: siteIcon('https://example.test/orders'),
    },
    {
      id: 't2',
      title: 'Docs',
      url: 'https://plain.test/docs',
      createdBy: { kind: 'user' },
      viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
      loading: false,
      canGoBack: false,
      canGoForward: false,
    },
  ]
}

/** The downloads the user started in each tab, as the Host follows them: a view that ends leaves them. */
interface GalleryDownloads {
  /** The downloads in `tab`, the newest last. */
  of(tab: string): LiveDownload[]
  /** The user's download in `tab`: it comes in for a moment, then the Host holds it under its name. */
  start(tab: string): void
}

/** One view of the gallery's conversation browser: a page it draws, and its controls. */
class GalleryBrowserView {
  private watched: string | null = null
  private generation = 0
  private sequence = 0
  private encoder: VideoEncoder | null = null
  private canvas = document.createElement('canvas')
  private painting: ReturnType<typeof setInterval> | null = null
  private heartbeat: ReturnType<typeof setInterval> | null = null
  private started = performance.now()
  private typed = 'Ship it'
  private selection: 'caret' | 'all' = 'caret'
  private pressed = false
  private overText = false
  private status = 'open'
  /** Whether the Host stopped capturing until the viewer asks again, as after a failed recreation of its extension. */
  private stopped = false

  constructor(
    private readonly handlers: UserStreamHandlers,
    /** The browser's tabs, shared with its tab requests. */
    private readonly tabs: LiveTab[],
    /** The number of the browser's latest tab list, as the Host numbers them. */
    private readonly list: () => number,
    /** Whether the Host can capture its tabs; one that cannot says so for each tab watched, as an Apple M4's Linux VM does. */
    private readonly capture: boolean,
    /** The watched page opens a tab, as a `target=_blank` link does. */
    private readonly pageOpens: (opener: string, url: string) => void,
    /** The browser's downloads, which outlive any view, as the Host follows them. */
    private readonly downloads: GalleryDownloads,
    /** Whether the Host holds back what its browser is doing, so the page has not read it yet. */
    private readonly unread: () => boolean,
    /** The viewer asked to capture again, which the Host does for every view. */
    private readonly recaptured: () => void,
  ) {
    this.heartbeat = setInterval(() => this.send({ type: 'heartbeat' }), 250)
    queueMicrotask(() => this.state())
  }

  private send(value: LiveModuleMessage): void {
    this.handlers.data(message(value))
  }

  /** Even recreating the capture extension failed: the pictures stop until the viewer's Retry. */
  stopCapture(): void {
    this.stopped = true
    this.restart()
  }

  /** The Host recreated its capture extension: the pictures come back. */
  resumeCapture(): void {
    this.stopped = false
    this.restart()
  }

  /** Something the Host could not do for this viewer, as the module tells it. */
  notice(code: string, text: string): void {
    this.send({ type: 'notice', code, message: text })
  }

  /** The tabs changed by a request: every view says so, and a view of a closed tab watches nothing. */
  state(): void {
    if (this.unread()) {
      return
    }
    if (this.watched !== null && !this.tabs.some((tab) => tab.id === this.watched)) {
      this.watched = null
      this.restart()
    }
    // A browser with no tabs does not run, as the Host's retires with its last tab.
    this.send({ type: 'state', running: this.tabs.length > 0, list: this.list(), tabs: this.tabs, watched: this.watched })
  }

  private tab(): LiveTab | null {
    return this.tabs.find((tab) => tab.id === this.watched) ?? null
  }

  /** The viewer's messages, as a module reads them. */
  receive(bytes: Uint8Array): void {
    const view = new DataView(bytes.buffer, bytes.byteOffset)
    if (bytes.length < 5 || bytes[4] !== LIVE_CONTROL_FRAME) {
      return
    }
    const length = view.getUint32(0)
    const value = JSON.parse(decoder.decode(bytes.subarray(5, 4 + length))) as LiveViewerMessage
    this.handle(value)
  }

  private handle(value: LiveViewerMessage): void {
    const tab = this.tab()
    switch (value.type) {
      case 'watch':
        this.watched = value.tab
        this.restart()
        this.state()
        break
      case 'panel': {
        const watched = this.tab()
        if (watched && watched.viewport.mode === 'web') {
          watched.viewport = {
            width: value.width,
            height: value.height,
            devicePixelRatio: value.devicePixelRatio,
            mode: 'web',
          }
          this.restart()
          this.state()
        }
        break
      }
      case 'mode': {
        const chosen = this.tabs.find((item) => item.id === value.tab)
        if (chosen) {
          chosen.viewport = value.mode === 'mobile'
            ? { width: 390, height: 844, devicePixelRatio: chosen.viewport.devicePixelRatio, mode: 'mobile' }
            : { ...chosen.viewport, width: 800, height: 600, mode: 'web' }
          this.restart()
          this.state()
        }
        break
      }
      case 'pointer':
        if (value.action === 'down' && tab && value.button === 'right') {
          // The page leaves the right click to the browser: what lies under it builds the menu.
          const link = within(INVOICE, value.x, value.y) ? INVOICE.url : within(DOWNLOAD, value.x, value.y) ? DOWNLOAD.url : ''
          this.send({
            type: 'menu',
            tab: tab.id,
            menu: {
              x: value.x,
              y: value.y,
              link,
              selection: this.selection === 'all',
              editable: value.x >= 24 && value.x <= tab.viewport.width - 24 && value.y >= 216 && value.y <= 252,
            },
          })
          break
        }
        // A link acts on the click, once the button is up, as a browser's does.
        if (value.action === 'up' && value.button === 'left' && tab && within(INVOICE, value.x, value.y)) {
          this.pageOpens(tab.id, INVOICE.url)
          break
        }
        if (value.action === 'up' && value.button === 'left' && tab && within(DOWNLOAD, value.x, value.y)) {
          this.downloads.start(tab.id)
          break
        }
        if (value.action === 'down' && tab) {
          this.pressed = value.y > 96 && value.y < 136 && value.x > 24 && value.x < 160
          if (this.pressed) {
            this.send({ type: 'dialog', tab: tab.id, dialog: { type: 'confirm', message: 'Ship order 4711?', defaultText: '' } })
          }
        }
        // The observer resolves only what the page leaves to the browser, as text under the pointer; the
        // regions decide the rest in the view, without a round trip.
        if (tab && overText(value.x, value.y) !== this.overText) {
          this.overText = overText(value.x, value.y)
          this.send({ type: 'cursor', tab: tab.id, cursor: this.overText ? 'text' : 'default', editable: false })
        }
        break
      case 'key':
        if (value.action !== 'down') {
          break
        }
        if ((value.modifiers & (CONTROL | META)) !== 0 && !value.altGraph && value.key.toLowerCase() === 'a') {
          this.selection = 'all'
        } else if ((value.modifiers & (CONTROL | META)) !== 0 && ['c', 'x'].includes(value.key.toLowerCase())) {
          // The page copies or cuts what is selected, and the Host hands it to the viewer's clipboard.
          if (this.selection === 'all') {
            this.send({ type: 'clipboard', text: this.typed })
            if (value.key.toLowerCase() === 'x') {
              this.typed = ''
              this.selection = 'caret'
            }
          }
        } else if (value.text) {
          this.handle({ type: 'text', tab: value.tab, text: value.text })
        } else if (value.key === 'Backspace') {
          this.typed = this.selection === 'all' ? '' : this.typed.slice(0, -1)
          this.selection = 'caret'
        }
        break
      case 'text':
        this.typed = (this.selection === 'all' ? '' : this.typed) + value.text
        this.selection = 'caret'
        break
      case 'paste':
        this.handle({ type: 'text', tab: value.tab, text: value.text })
        break
      case 'choice':
        this.status = value.value
        this.send({ type: 'choice', token: value.token, accepted: true })
        break
      case 'dialog':
        if (tab) {
          this.send({ type: 'dialog', tab: tab.id, dialog: null })
        }
        this.pressed = false
        break
      case 'keyframe':
        this.restart()
        break
      case 'recapture':
        this.recaptured()
        break
      default:
        break
    }
  }

  /** The downloads in `tab` changed: a view watching it hears the list, as the module tells its viewer. */
  downloadsChanged(tab: string): void {
    if (this.watched === tab) {
      this.send({ type: 'downloads', tab, downloads: this.downloads.of(tab) })
    }
  }

  private restart(): void {
    this.stopPictures()
    const tab = this.tab()
    if (!tab) {
      return
    }
    // The page's own pixels, as the Host captures them; an odd side loses its last row or column.
    const size = {
      width: even(Math.ceil(tab.viewport.width * tab.viewport.devicePixelRatio)),
      height: even(Math.ceil(tab.viewport.height * tab.viewport.devicePixelRatio)),
    }
    this.generation += 1
    this.sequence = 0
    this.canvas.width = size.width
    this.canvas.height = size.height
    this.send({ type: 'controls', tab: tab.id, controls: [{ ...SELECT, value: this.status }] })
    this.send({ type: 'cursors', tab: tab.id, regions: pageCursors(tab.viewport) })
    this.send({ type: 'downloads', tab: tab.id, downloads: this.downloads.of(tab.id) })
    if (!this.capture) {
      this.notice(LIVE_CAPTURE_UNAVAILABLE, 'this CPU reports SME without SVE, and Chrome cannot capture on it')
      return
    }
    if (this.stopped) {
      this.notice(LIVE_CAPTURE_STOPPED, 'the capture extension could not be recreated')
      return
    }
    // A real Host can report controls before its encoder starts a generation.
    this.send({
      type: 'stream',
      tab: tab.id,
      generation: this.generation,
      width: size.width,
      height: size.height,
      viewport: { ...tab.viewport },
      scale: 1,
    })
    const generation = this.generation
    this.encoder = new VideoEncoder({
      output: (chunk) => {
        if (generation !== this.generation) {
          return
        }
        this.sequence += 1
        this.handlers.data(picture(tab.id, generation, this.sequence, chunk, size))
      },
      error: () => this.stopPictures(),
    })
    this.encoder.configure({
      codec: LIVE_VIDEO_CODEC,
      width: size.width,
      height: size.height,
      framerate: FPS,
      bitrate: 4_000_000,
      latencyMode: 'realtime',
      hardwareAcceleration: 'prefer-software',
      avc: { format: 'annexb' },
    })
    let frames = 0
    this.painting = setInterval(() => {
      this.paint(tab)
      const frame = new VideoFrame(this.canvas, {
        timestamp: Math.round((performance.now() - this.started) * 1000),
      })
      try {
        this.encoder?.encode(frame, { keyFrame: frames % (FPS * 2) === 0 })
      } finally {
        frame.close()
        frames += 1
      }
    }, 1000 / FPS)
  }

  /** A page worth looking at: a heading, a button, a field and a select; a blank page on `about:blank`, as a new tab is. */
  private paint(tab: LiveTab): void {
    const context = this.canvas.getContext('2d')
    if (!context) {
      return
    }
    const viewport = tab.viewport
    const ratio = viewport.devicePixelRatio
    context.setTransform(ratio, 0, 0, ratio, 0, 0)
    // The Host's Chrome runs in the user's scheme, here the gallery's: it paints an empty page #121212 in
    // the dark one, and a page of its own, which declares no scheme, on white.
    const dark = document.documentElement.getAttribute('data-theme') === 'dark'
    context.fillStyle = tab.url === 'about:blank' && dark ? '#121212' : '#ffffff'
    context.fillRect(0, 0, viewport.width, viewport.height)
    if (tab.url === 'about:blank') {
      return
    }
    context.fillStyle = '#0f172a'
    context.font = '600 22px system-ui, sans-serif'
    context.fillText('Orders', 24, 48)
    context.font = '14px system-ui, sans-serif'
    context.fillStyle = '#475569'
    context.fillText('Order 4711 · 3 items · ready to ship', 24, 76)
    context.fillStyle = this.pressed ? '#1d4ed8' : '#2563eb'
    context.fillRect(24, 96, 136, 40)
    context.fillStyle = '#ffffff'
    context.font = '600 14px system-ui, sans-serif'
    context.fillText('Ship order', 44, 121)
    context.strokeStyle = '#cbd5e1'
    context.strokeRect(24, 168, 180, 32)
    context.fillStyle = '#0f172a'
    context.font = '14px system-ui, sans-serif'
    context.fillText(this.status === 'open' ? 'Open' : 'Shipped', 34, 189)
    context.strokeRect(24, 216, viewport.width - 48, 36)
    if (this.selection === 'all') {
      context.fillStyle = '#bfdbfe'
      context.fillRect(32, 220, context.measureText(this.typed).width + 4, 28)
      context.fillStyle = '#0f172a'
    }
    context.fillText(this.typed, 34, 239)
    context.fillStyle = '#2563eb'
    context.fillText('Open the invoice in a new tab', INVOICE.x, INVOICE.y + 14)
    context.fillText('Download guide.pdf', DOWNLOAD.x, DOWNLOAD.y + 14)
    // A moving mark, so a still picture is told from a stalled one.
    const seconds = (performance.now() - this.started) / 1000
    context.fillStyle = '#22c55e'
    context.beginPath()
    context.arc(
      40 + ((seconds * 60) % Math.max(40, viewport.width - 80)),
      viewport.height - 48,
      12,
      0,
      Math.PI * 2,
    )
    context.fill()
  }

  private stopPictures(): void {
    if (this.painting !== null) {
      clearInterval(this.painting)
      this.painting = null
    }
    if (this.encoder && this.encoder.state !== 'closed') {
      this.encoder.close()
    }
    this.encoder = null
  }

  stop(): void {
    this.stopPictures()
    if (this.heartbeat !== null) {
      clearInterval(this.heartbeat)
      this.heartbeat = null
    }
    this.handlers.closed('closed')
  }
}

/**
 * The gallery's conversation browser, as the Host runs it (`live-view.md`):
 * its tabs and the operations the plugin runs on them, and the live view's
 * stream over the same tabs, without a Host. Operations take a moment, as a
 * Host does, and a page it loads takes a while longer, so the page's loading
 * shows. It starts with the agent's and the user's tab unless a specimen
 * supplies its own list, such as an empty one for a panel whose strip starts
 * empty. Without `chrome`, the Host lacks the browser, which only the agent installs, so the
 * strip offers no new tab and says why; without `capture`, the Host cannot capture its tabs, so
 * a view shows no picture and says why.
 */
export interface GalleryBrowser {
  /** The tab list, as the plugin's conversation state last brought it. */
  listed: ShallowRef<BrowserTabList>
  open(url: string): Promise<BrowserTab>
  /** Closes the tab; a tab the browser does not have is closed already. */
  close(tab: string): Promise<void>
  /** Answers the number of the last tab list before the request started, as a user's navigation on the Host does. */
  navigate(tab: string, url: string): Promise<number>
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<number>
  /** Stops loading the tab's page; answers as `navigate` does. */
  stop(tab: string): Promise<number>
  /** The tabs closed on purpose, which the panel removes rather than open again. */
  closedOnPurpose: ReadonlySet<string>
  /** The tab closes on the device on purpose, as the agent's `demi browser close` closes it. */
  closeOnDevice(tab: string): void
  /** The browser ends on the device and takes its tabs with it, as a release or a stopped Cloud does. */
  end(): void
  /** The next tab the browser opens is refused, as an offline device refuses it. */
  failNextOpen(): void
  /** The agent shows the tab to the user, as `demi browser show` does: its count of showings rises. */
  show(tab: string): void
  /** The agent opens a tab, as `demi browser open` does, and shows it with `show`, as `--show` does. */
  agentOpens(url: string, options: { show: boolean }): void
  /** The Host fails what a viewer asked, such as its input, and tells every view, as the module's notice does. */
  fail(code: string, message: string): void
  /** Even recreating the capture extension fails on the Host: every view loses its pictures and says so, until Retry. */
  stopCapture(): void
  stream: OpenUserStream
  /** What the Host holds of the browser's package. */
  installed(): readonly HostArtifact[]
  /** Whether the Host is a Cloud that is starting, as the product state says while it wakes. */
  hostStarting: ShallowRef<boolean>
  /** The Cloud stops, as an idle Cloud does, and its browser ends with it; the next tab opened or shown starts it again. */
  stopCloud(): void
  /** What a held opening waits for, and nothing once `finish` let it go on. */
  held: ShallowRef<StartingPhase | null>
  /** An opening held where `held` says goes on, and the next ones do not wait there. */
  finish(): void
  /** The next openings wait in `phase` until `finish`. */
  hold(phase: StartingPhase): void
}

/**
 * The gallery's conversation browser. `held` keeps every opening in one
 * phase before the browser has its tab, for a specimen that shows it, until
 * `finish`: Connecting… while the page has read neither the plugin's tab
 * list nor a view's state, Starting Cloud… on a Cloud that starts, Starting
 * the browser… on a Host whose browser has no tabs yet, or Opening the
 * page….
 */
export function galleryBrowser(
  tabs: LiveTab[] = galleryTabs(),
  { chrome = true, capture = true, held: holding = null }: { chrome?: boolean; capture?: boolean; held?: StartingPhase | null } = {},
): GalleryBrowser {
  const views = new Set<GalleryBrowserView>()
  // The next tab's number, as the conversation gives them: never one given before.
  let next = Math.max(0, ...tabs.map((tab) => Number(tab.id.slice(1)))) + 1

  /** Answers after a Host's moment; the timer ends by itself within 900 ms. */
  function later<T>(answer: () => T): Promise<T> {
    return new Promise((resolve, reject) => {
      setTimeout(() => {
        try {
          resolve(answer())
        } catch (error) {
          reject(error)
        }
      }, REQUEST_DELAY_MS)
    })
  }

  /** How many times the agent showed each tab, as the Host's tab registry counts them. */
  const shows = new Map<string, number>()

  function info(tab: LiveTab): BrowserTab {
    return {
      id: tab.id,
      title: tab.title,
      url: tab.url,
      createdBy: tab.createdBy,
      loading: tab.loading,
      canGoBack: tab.canGoBack,
      canGoForward: tab.canGoForward,
      shows: shows.get(tab.id) ?? 0,
      ...(tab.favicon ? { favicon: tab.favicon } : {}),
    }
  }

  /** The browser a tab needs, which the pinned Chrome for Testing is. */
  const needed = { name: 'Chrome for Testing', version: '153.0.8010.36' }
  const artifacts: readonly HostArtifact[] = [
    { package: 'demi.browser', name: 'program', version: '0.1.3' },
    ...(chrome ? [{ package: 'demi.browser', ...needed }] : []),
  ]
  const listed = shallowRef<BrowserTabList>({ tabs: tabs.map(info), browser: needed })

  /** Each change of the tabs is a new tab list, numbered as the Host numbers them. */
  let lists = 0

  function changed(): void {
    lists += 1
    listed.value = { tabs: tabs.map(info), browser: needed }
    for (const view of views) {
      view.state()
    }
  }

  function found(id: string): LiveTab {
    const tab = tabs.find((item) => item.id === id)
    if (!tab) {
      throw new BrowserTabsError('tab_not_found', 'The browser has no such tab')
    }
    return tab
  }

  /** Each tab's history, as the browser keeps it: its addresses, and the one it shows. */
  const histories = new Map<string, { entries: string[]; index: number }>()

  function historyOf(tab: LiveTab): { entries: string[]; index: number } {
    let history = histories.get(tab.id)
    if (!history) {
      history = { entries: [tab.url], index: 0 }
      histories.set(tab.id, history)
    }
    return history
  }

  /** Each tab's load on its way: its start, then its end. A Stop clears it; otherwise it ends by itself. */
  const loads = new Map<string, ReturnType<typeof setTimeout>>()

  /**
   * `tab` starts loading `url` a moment after the request's answer, where
   * `commit` moves its history. Answers the number of the last tab list
   * before then.
   */
  function loadLater(tab: LiveTab, url: string, commit: () => void = () => {}): number {
    clearTimeout(loads.get(tab.id))
    loads.set(tab.id, setTimeout(() => {
      commit()
      load(tab, url)
    }, LOAD_START_MS))
    return lists
  }

  /** `tab` loads `url`: it says so until the page is there. */
  function load(tab: LiveTab, url: string): void {
    // A page loaded again keeps its title; another is named by its host until it says otherwise.
    if (url !== tab.url) {
      tab.title = URL.parse(url)?.host ?? url
    }
    tab.url = url
    tab.loading = url !== 'about:blank'
    const history = historyOf(tab)
    tab.canGoBack = history.index > 0
    tab.canGoForward = history.index < history.entries.length - 1
    // The site's icon arrives once its page has loaded.
    if (!tab.loading) {
      tab.favicon = siteIcon(url)
    }
    changed()
    if (tab.loading) {
      loads.set(tab.id, setTimeout(() => {
        loads.delete(tab.id)
        tab.loading = false
        tab.title = PAGE_TITLES.get(url) ?? tab.title
        tab.favicon = siteIcon(url)
        changed()
      }, LOAD_MS))
    }
  }

  /** The tabs closed on purpose, as the Host's tab list names them. */
  const closedOnPurpose = new Set<string>()

  /** The downloads the user started in each tab, kept while views come and go, as the Host keeps them. */
  const downloaded = new Map<string, LiveDownload[]>()
  const downloads: GalleryDownloads = {
    of: (tab) => [...(downloaded.get(tab) ?? [])],
    start: (tab) => {
      const download: LiveDownload = { id: crypto.randomUUID(), ...DOWNLOADED, state: 'inProgress', received: 0, path: '' }
      downloaded.set(tab, [...(downloaded.get(tab) ?? []), download])
      for (const view of views) {
        view.downloadsChanged(tab)
      }
      // The timer ends by itself once the file is in.
      setTimeout(() => {
        Object.assign(download, { state: 'complete', received: DOWNLOADED.total, path: DOWNLOADED.path })
        for (const view of views) {
          view.downloadsChanged(tab)
        }
      }, DOWNLOAD_MS)
    },
  }

  /** Whether the Host stopped capturing, as after a failed recreation of its extension, until a viewer's Retry. */
  let captureStopped = false
  /** A viewer's Retry: the Host recreates its extension, and every view's pictures come back. */
  function recaptured(): void {
    captureStopped = false
    for (const view of views) {
      view.resumeCapture()
    }
  }

  const stream: OpenUserStream = (handlers) => {
    const browser = new GalleryBrowserView(
      handlers,
      tabs,
      () => lists,
      capture,
      pageOpens,
      downloads,
      () => held.value === 'connecting',
      recaptured,
    )
    views.add(browser)
    // A Host that stopped capturing says so to a view that opens meanwhile.
    if (captureStopped) {
      browser.stopCapture()
    }
    return {
      send: (bytes) => browser.receive(bytes),
      close: () => {
        views.delete(browser)
        browser.stop()
      },
    }
  }

  function remove(id: string): void {
    const index = tabs.findIndex((item) => item.id === id)
    clearTimeout(loads.get(id))
    loads.delete(id)
    if (index >= 0) {
      closedOnPurpose.add(id)
      tabs.splice(index, 1)
      changed()
    }
  }

  /** Whether the next open is refused. */
  let refuseOpen = false

  const hostStarting = shallowRef(holding === 'cloud')
  const held = shallowRef<StartingPhase | null>(holding)
  /** The openings `held` keeps, until `finish`. */
  const waiting: Array<() => void> = []
  /** Whether the Cloud stopped, so the next opening starts it first. */
  let cloudStopped = false

  /** Waits `ms`; the timer ends by itself. */
  function wait(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms))
  }

  /**
   * What an opening waits for before the browser opens its tab: a held
   * phase until `finish`, a stopped Cloud while it starts, and a browser
   * with no tabs while it starts, each as the Host takes it.
   */
  async function started(): Promise<void> {
    if (held.value !== null) {
      await new Promise<void>((resolve) => waiting.push(resolve))
    }
    if (cloudStopped) {
      hostStarting.value = true
      await wait(CLOUD_START_MS)
      hostStarting.value = false
      cloudStopped = false
    }
    if (tabs.length === 0) {
      await wait(BROWSER_START_MS)
    }
  }

  /** The browser ends on the device and takes its tabs with it. */
  function end(): void {
    for (const timer of loads.values()) {
      clearTimeout(timer)
    }
    loads.clear()
    tabs.splice(0)
    changed()
  }

  /** A page opens a tab, as a `target=_blank` link or `window.open` does: it loads beside its opener. */
  function pageOpens(opener: string, url: string): void {
    const tab: LiveTab = {
      id: `t${next++}`,
      title: URL.parse(url)?.host ?? url,
      url,
      createdBy: { kind: 'page', opener },
      viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
      loading: false,
      canGoBack: false,
      canGoForward: false,
    }
    tabs.push(tab)
    load(tab, url)
  }

  /** The browser opens a tab on `url`, after a Host's moment; refused once `failNextOpen` said so. */
  function opened(url: string): Promise<BrowserTab> {
    return later(() => {
      if (refuseOpen) {
        refuseOpen = false
        throw new BrowserTabsError('device_offline', 'The device is offline')
      }
      const tab: LiveTab = {
        id: `t${next++}`,
        title: url === 'about:blank' ? 'about:blank' : URL.parse(url)?.host ?? url,
        url,
        createdBy: { kind: 'user' },
        viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
        loading: false,
        canGoBack: false,
        canGoForward: false,
      }
      tabs.push(tab)
      load(tab, url)
      return info(tab)
    })
  }

  return {
    listed,
    open: async (url) => {
      await started()
      return opened(url)
    },
    close: (id) => later(() => remove(id)),
    navigate: (id, url) => later(() => {
      const tab = found(id)
      // The Host's own refusal of an address no browser tab opens for the agent or the user.
      if (!['http:', 'https:', 'file:'].includes(URL.parse(url)?.protocol ?? '') && url !== 'about:blank') {
        throw new BrowserTabsError('invalid_input', 'navigation accepts http:, https:, file:, or about:blank')
      }
      const history = historyOf(tab)
      return loadLater(tab, url, () => {
        history.entries.splice(history.index + 1, Infinity, url)
        history.index = history.entries.length - 1
      })
    }),
    history: (id, action) => later(() => {
      const tab = found(id)
      if (action === 'reload') {
        return loadLater(tab, tab.url)
      }
      const history = historyOf(tab)
      const index = history.index + (action === 'back' ? -1 : 1)
      const url = history.entries[index]
      // The Host's own refusal at either end of the history.
      if (url === undefined) {
        throw new BrowserTabsError('history_boundary', 'no navigation entry in that direction')
      }
      return loadLater(tab, url, () => {
        history.index = index
      })
    }),
    stop: (id) => later(() => {
      const tab = found(id)
      // The list the Host read before it stopped the page; it reads the list again after.
      const before = lists
      clearTimeout(loads.get(id))
      loads.delete(id)
      tab.loading = false
      changed()
      return before
    }),
    closedOnPurpose,
    closeOnDevice: remove,
    end,
    failNextOpen: () => {
      refuseOpen = true
    },
    show: (id) => {
      shows.set(id, (shows.get(id) ?? 0) + 1)
      changed()
    },
    agentOpens: (url, options) => {
      const tab: LiveTab = {
        id: `t${next++}`,
        title: URL.parse(url)?.host ?? url,
        url,
        createdBy: { kind: 'agent', number: 0 },
        viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
        loading: false,
        canGoBack: false,
        canGoForward: false,
        favicon: siteIcon(url),
      }
      tabs.push(tab)
      if (options.show) {
        shows.set(tab.id, 1)
      }
      changed()
    },
    fail: (code, message) => {
      for (const view of views) {
        view.notice(code, message)
      }
    },
    stopCapture: () => {
      captureStopped = true
      for (const view of views) {
        view.stopCapture()
      }
    },
    stream,
    installed: () => artifacts,
    hostStarting,
    stopCloud: () => {
      cloudStopped = true
      end()
    },
    held,
    finish: () => {
      held.value = null
      hostStarting.value = false
      // A view held back while the page connected says what the browser holds now.
      for (const view of views) {
        view.state()
      }
      for (const resolve of waiting.splice(0)) {
        resolve()
      }
    },
    hold: (phase) => {
      held.value = phase
      hostStarting.value = phase === 'cloud'
      // The page connects again, as after a lost connection, and reads nothing until `finish`.
      if (phase === 'connecting') {
        for (const view of [...views]) {
          view.stop()
        }
      }
    },
  }
}

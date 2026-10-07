/**
 * A live browser view without a Host (`live-view.md`): the gallery
 * draws a page, encodes it as the Host's capture would, and speaks the live
 * protocol, so the view's pictures, input, controls and dialogs show here.
 */
import { LIVE_CONTROL_FRAME, LIVE_VIDEO_CODEC } from '@demicodes/plugin-browser/generated/plugin'
import type {
  BrowserTab,
  BrowserViewport,
  CursorRegion,
  LiveControl,
  LiveModuleMessage,
  LiveTab,
  LiveViewerMessage,
} from '@demicodes/plugin-browser/generated/plugin'
import { shallowRef, type ShallowRef } from 'vue'
import { encodeVideo } from '@demicodes/plugin-browser/live/frames'
import type { OpenUserStream, UserStreamHandlers } from '@demicodes/web-ui/plugins/streams'
import { CONTROL, META } from '@demicodes/plugin-browser/live/input'
import { BrowserTabsError, type BrowserTabList } from '@demicodes/plugin-browser/live/tabs'
import type { HostArtifact } from '@demicodes/web-ui/devices/installed'

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

/** Where the drawn page shows which cursor, as a page's observer reports it: the button, the select and the field. */
function pageCursors(viewport: BrowserViewport): CursorRegion[] {
  return [
    { x: 24, y: 96, width: 136, height: 40, cursor: 'pointer' },
    { x: 24, y: 168, width: 180, height: 32, cursor: 'default' },
    { x: 24, y: 216, width: viewport.width - 48, height: 36, cursor: 'text' },
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
/** How long a page the gallery's browser loads takes, so the page's loading shows. */
const LOAD_MS = 1200
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
    },
    {
      id: 't2',
      title: 'Docs',
      url: 'https://example.test/docs',
      createdBy: { kind: 'user' },
      viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
      loading: false,
    },
  ]
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

  constructor(
    private readonly handlers: UserStreamHandlers,
    /** The browser's tabs, shared with its tab requests. */
    private readonly tabs: LiveTab[],
  ) {
    this.heartbeat = setInterval(() => this.send({ type: 'heartbeat' }), 250)
    queueMicrotask(() => this.state())
  }

  private send(value: LiveModuleMessage): void {
    this.handlers.data(message(value))
  }

  /** The tabs changed by a request: every view says so, and a view of a closed tab watches nothing. */
  state(): void {
    if (this.watched !== null && !this.tabs.some((tab) => tab.id === this.watched)) {
      this.watched = null
      this.restart()
    }
    this.send({ type: 'state', running: true, tabs: this.tabs, watched: this.watched })
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
      default:
        break
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
      this.paint(tab.viewport)
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

  /** A page worth looking at: a heading, a button, a field and a select. */
  private paint(viewport: BrowserViewport): void {
    const context = this.canvas.getContext('2d')
    if (!context) {
      return
    }
    const ratio = viewport.devicePixelRatio
    context.setTransform(ratio, 0, 0, ratio, 0, 0)
    context.fillStyle = '#ffffff'
    context.fillRect(0, 0, viewport.width, viewport.height)
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
 * strip offers no new tab and says why.
 */
export interface GalleryBrowser {
  /** The tab list, as the plugin's conversation state last brought it. */
  listed: ShallowRef<BrowserTabList>
  open(url: string): Promise<BrowserTab>
  /** Closes the tab; a tab the browser does not have is closed already. */
  close(tab: string): Promise<void>
  navigate(tab: string, url: string): Promise<void>
  history(tab: string, action: 'back' | 'forward' | 'reload'): Promise<void>
  /** The tab closes on the device, as the agent's close or a browser that ended would close it. */
  closeOnDevice(tab: string): void
  /** The agent shows the tab to the user, as `demi browser show` does: its count of showings rises. */
  show(tab: string): void
  /** The agent opens a tab, as `demi browser open` does, and shows it with `show`, as `--show` does. */
  agentOpens(url: string, options: { show: boolean }): void
  stream: OpenUserStream
  /** What the Host holds of the browser's package. */
  installed(): readonly HostArtifact[]
}

export function galleryBrowser(
  tabs: LiveTab[] = galleryTabs(),
  { chrome = true }: { chrome?: boolean } = {},
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
    return { id: tab.id, title: tab.title, url: tab.url, createdBy: tab.createdBy, loading: tab.loading, shows: shows.get(tab.id) ?? 0 }
  }

  /** The browser a tab needs, which the pinned Chrome for Testing is. */
  const needed = { name: 'Chrome for Testing', version: '153.0.8010.36' }
  const held: readonly HostArtifact[] = [
    { package: 'demi.browser', name: 'program', version: '0.1.3' },
    ...(chrome ? [{ package: 'demi.browser', ...needed }] : []),
  ]
  const listed = shallowRef<BrowserTabList>({ tabs: tabs.map(info), browser: needed })

  function changed(): void {
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

  /** `tab` starts loading `url` a moment after the request's answer. The timer ends by itself. */
  function loadLater(tab: LiveTab, url: string): void {
    setTimeout(() => load(tab, url), LOAD_START_MS)
  }

  /** `tab` loads `url`: it says so until the page is there. The timer ends by itself. */
  function load(tab: LiveTab, url: string): void {
    // A page loaded again keeps its title; another is named by its host until it says otherwise.
    if (url !== tab.url) {
      tab.title = URL.parse(url)?.host ?? url
    }
    tab.url = url
    tab.loading = url !== 'about:blank'
    changed()
    if (tab.loading) {
      setTimeout(() => {
        tab.loading = false
        changed()
      }, LOAD_MS)
    }
  }

  const stream: OpenUserStream = (handlers) => {
    const browser = new GalleryBrowserView(handlers, tabs)
    views.add(browser)
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
    if (index >= 0) {
      tabs.splice(index, 1)
      changed()
    }
  }

  return {
    listed,
    open: (url) => later(() => {
      const tab: LiveTab = {
        id: `t${next++}`,
        title: url === 'about:blank' ? 'about:blank' : URL.parse(url)?.host ?? url,
        url,
        createdBy: { kind: 'user' },
        viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
        loading: false,
      }
      tabs.push(tab)
      load(tab, url)
      return info(tab)
    }),
    close: (id) => later(() => remove(id)),
    navigate: (id, url) => later(() => {
      const tab = found(id)
      const history = historyOf(tab)
      history.entries.splice(history.index + 1, Infinity, url)
      history.index = history.entries.length - 1
      loadLater(tab, url)
    }),
    history: (id, action) => later(() => {
      const tab = found(id)
      if (action === 'reload') {
        loadLater(tab, tab.url)
        return
      }
      const history = historyOf(tab)
      const index = history.index + (action === 'back' ? -1 : 1)
      const url = history.entries[index]
      // The Host's own refusal at either end of the history.
      if (url === undefined) {
        throw new BrowserTabsError('history_boundary', 'no navigation entry in that direction')
      }
      history.index = index
      loadLater(tab, url)
    }),
    closeOnDevice: remove,
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
      }
      tabs.push(tab)
      if (options.show) {
        shows.set(tab.id, 1)
      }
      changed()
    },
    stream,
    installed: () => held,
  }
}

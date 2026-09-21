/**
 * A live browser view without a Host (`browser-live-view.md`): the gallery
 * draws a page, encodes it as the Host's capture would, and speaks the live
 * protocol, so the view's pictures, input, controls and dialogs show here.
 */
import {
  LIVE_FRAME_KIND,
  type LiveControl,
  type LiveModuleMessage,
  type LiveTab,
  type LiveViewerMessage,
  type LiveViewport,
} from '@demicodes/browser-protocol/live'
import type { OpenLiveStream, LiveStreamHandlers } from '@demicodes/web-ui/browser/session'
import { BrowserTabsError, type BrowserTabInfo, type BrowserTabsApi } from '@demicodes/web-ui/browser/tabs'

const CODEC = 'avc1.640033'
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
  return framed(LIVE_FRAME_KIND.control, encoder.encode(JSON.stringify(value)))
}

function picture(
  tab: string,
  generation: number,
  sequence: number,
  chunk: EncodedVideoChunk,
  size: { width: number; height: number },
): Uint8Array {
  const payload = new Uint8Array(48 + chunk.byteLength)
  const header = new DataView(payload.buffer)
  payload.set(encoder.encode(tab).subarray(0, 24))
  header.setUint32(24, generation)
  header.setUint32(28, sequence)
  header.setUint8(32, chunk.type === 'key' ? 1 : 0)
  header.setFloat64(36, chunk.timestamp)
  header.setUint16(44, size.width)
  header.setUint16(46, size.height)
  chunk.copyTo(payload.subarray(48))
  return framed(LIVE_FRAME_KIND.video, payload)
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

/** How long the gallery's browser takes over a request, as a Host takes a moment. */
const REQUEST_DELAY_MS = 900

function galleryTabs(): LiveTab[] {
  return [
    {
      id: 't_galleryaaaaaaaaaaaaaa',
      title: 'Orders — Example',
      url: 'https://example.test/orders',
      createdBy: { kind: 'agent', nodeId: 'root' },
      viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
    },
    {
      id: 't_gallerybbbbbbbbbbbbbb',
      title: 'Docs',
      url: 'https://example.test/docs',
      createdBy: { kind: 'user' },
      viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
    },
  ]
}

/** One view of the gallery's browser: a page it draws, and its controls. */
class GalleryBrowser {
  private watched: string | null = null
  private generation = 0
  private sequence = 0
  private encoder: VideoEncoder | null = null
  private canvas = document.createElement('canvas')
  private painting: ReturnType<typeof setInterval> | null = null
  private heartbeat: ReturnType<typeof setInterval> | null = null
  private started = performance.now()
  private typed = 'Ship it'
  private pressed = false
  private status = 'open'

  constructor(
    private readonly handlers: LiveStreamHandlers,
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
    if (bytes.length < 5 || bytes[4] !== LIVE_FRAME_KIND.control) {
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
        this.send({
          type: 'cursor',
          tab: tab?.id ?? '',
          cursor: value.y > 96 && value.y < 136 ? 'pointer' : 'default',
          editable: false,
        })
        break
      case 'key':
        if (value.action === 'down' && value.text) {
          this.typed += value.text
        } else if (value.action === 'down' && value.key === 'Backspace') {
          this.typed = this.typed.slice(0, -1)
        }
        break
      case 'text':
        this.typed += value.text
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
    const size = {
      width: Math.ceil(tab.viewport.width * tab.viewport.devicePixelRatio / 2) * 2,
      height: Math.ceil(tab.viewport.height * tab.viewport.devicePixelRatio / 2) * 2,
    }
    this.generation += 1
    this.sequence = 0
    this.canvas.width = size.width
    this.canvas.height = size.height
    this.send({ type: 'stream', tab: tab.id, generation: this.generation, width: size.width, height: size.height })
    this.send({ type: 'controls', tab: tab.id, controls: [{ ...SELECT, value: this.status }] })
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
      codec: CODEC,
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
  private paint(viewport: LiveViewport): void {
    const context = this.canvas.getContext('2d')
    if (!context) {
      return
    }
    const ratio = this.canvas.width / viewport.width
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
 * The gallery's conversation browser: the tab requests a `browser` tab kind
 * makes and the view it opens, over one tab list, without a Host. Requests
 * take a moment, as a Host does, so the content's waiting shows.
 */
export function galleryBrowserTabs(): BrowserTabsApi {
  const tabs = galleryTabs()
  const views = new Set<GalleryBrowser>()
  // Each request's timer removes itself when it answers; none outlives its 900 ms.
  const timers = new Set<ReturnType<typeof setTimeout>>()

  function later<T>(answer: () => T): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        timers.delete(timer)
        try {
          resolve(answer())
        } catch (error) {
          reject(error)
        }
      }, REQUEST_DELAY_MS)
      timers.add(timer)
    })
  }

  function changed(): void {
    for (const view of views) {
      view.state()
    }
  }

  function info(tab: LiveTab): BrowserTabInfo {
    return { id: tab.id, title: tab.title, url: tab.url, createdBy: tab.createdBy }
  }

  function found(id: string): LiveTab {
    const tab = tabs.find((item) => item.id === id)
    if (!tab) {
      throw new BrowserTabsError('tab_not_found', 'The browser has no such tab')
    }
    return tab
  }

  const stream: OpenLiveStream = (handlers) => {
    const browser = new GalleryBrowser(handlers, tabs)
    views.add(browser)
    return {
      send: (bytes) => browser.receive(bytes),
      close: () => {
        views.delete(browser)
        browser.stop()
      },
    }
  }

  return {
    list: () => later(() => ({ tabs: tabs.map(info) })),
    open: (url) => later(() => {
      const tab: LiveTab = {
        id: `t_gallery${Math.random().toString(36).slice(2).padEnd(14, '0').slice(0, 14)}`,
        title: url === 'about:blank' ? '' : URL.parse(url)?.host ?? url,
        url,
        createdBy: { kind: 'user' },
        viewport: { width: 800, height: 600, devicePixelRatio: 2, mode: 'web' },
      }
      tabs.push(tab)
      changed()
      return info(tab)
    }),
    close: (id) => later(() => {
      const index = tabs.findIndex((item) => item.id === id)
      if (index >= 0) {
        tabs.splice(index, 1)
        changed()
      }
    }),
    navigate: (id, url) => later(() => {
      const tab = found(id)
      tab.url = url
      tab.title = URL.parse(url)?.host ?? url
      changed()
    }),
    // The gallery's pages have no history of their own; every action answers as done.
    history: (id) => later(() => void found(id)),
    stream,
  }
}

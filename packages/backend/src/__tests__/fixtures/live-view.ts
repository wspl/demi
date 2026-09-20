/**
 * The pages and the page-side client of the live browser acceptance
 * (`browser-live-view.md` § Acceptance), shared by the paired-device run and
 * the Cloud run: fixture pages that record what they received, and a viewer
 * that speaks the live protocol over a conversation's stream.
 */
import { waitFor } from '@demicodes/utils'

/** Stripes half a CSS pixel wide: flat grey at ratio 1, black and white above. */
const page = (paintMs: number) => `<!doctype html>
<meta name="viewport" content="width=device-width">
<style>
  body { margin: 0; height: 2000px; font: 16px system-ui, sans-serif }
  #stripes { position: absolute; left: 0; top: 0; width: 400px; height: 80px;
    background: repeating-linear-gradient(90deg, #000 0, #000 0.5px, #fff 0.5px, #fff 1px) }
  #field { position: absolute; left: 20px; top: 120px; width: 320px; height: 32px }
  #area { position: absolute; left: 20px; top: 170px; width: 320px; height: 60px }
  #spin { position: absolute; left: 0; top: 300px; width: 40px; height: 40px; background: #e11d48 }
</style>
<div id="stripes"></div>
<input id="field">
<textarea id="area"></textarea>
<div id="spin"></div>
<script>
const record = what => navigator.sendBeacon('/record', JSON.stringify(what))
for (const type of ['keydown', 'keypress', 'input', 'change', 'paste', 'mousedown', 'mouseup', 'wheel'])
  addEventListener(type, event => record({
    type,
    key: event.key ?? null,
    target: event.target.id ?? '',
    value: event.target.value ?? null,
    scrollY: Math.round(scrollY),
    text: type === 'paste' ? event.clipboardData.getData('text/plain') : null,
  }), { capture: true, passive: true })
const report = () => record({ type: 'ready', devicePixelRatio, inner: [innerWidth, innerHeight],
  webdriver: navigator.webdriver ?? null, headless: /Headless/.test(navigator.userAgent),
  hover: matchMedia('(hover: hover)').matches, finePointer: matchMedia('(pointer: fine)').matches,
  timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone, language: navigator.language,
  scrollbar: innerWidth - document.documentElement.clientWidth })
// The viewer's panel and its screen decide these; report them as they change.
report()
addEventListener('resize', report)
const density = () => {
  report()
  matchMedia('(resolution: ' + devicePixelRatio + 'dppx)').addEventListener('change', density, { once: true })
}
density()
let x = 0
setInterval(() => { x = x > 300 ? 0 : x + 6; spin.style.left = x + 'px' }, ${paintMs})
</script>`

/**
 * Chinese, Japanese and Korean text: the samples are pairs of distinct
 * characters. A font that has them draws each one differently; a font that
 * lacks them draws every one as the same missing-character box.
 */
const FONTS_PAGE = `<!doctype html>
<meta charset="utf-8">
<style>body { margin: 0; font: 32px system-ui, sans-serif }</style>
<div id="text">中文 日本語 한국어</div>
<script>
const draw = character => {
  const canvas = document.createElement('canvas')
  canvas.width = canvas.height = 48
  const context = canvas.getContext('2d')
  context.font = '32px sans-serif'
  context.fillText(character, 4, 38)
  const pixels = context.getImageData(0, 0, 48, 48).data
  let ink = 0
  let signature = 0
  for (let at = 3; at < pixels.length; at += 4) {
    ink += pixels[at] > 32 ? 1 : 0
    signature = (signature * 31 + pixels[at]) >>> 0
  }
  return { ink, signature }
}
navigator.sendBeacon('/record', JSON.stringify({
  type: 'fonts',
  samples: ['中国', 'あい', '한국'].map(pair => ({
    pair,
    first: draw(pair[0]),
    second: draw(pair[1]),
  })),
}))
</script>`

export interface LiveRecord {
  type: string
  key?: string | null
  target?: string
  value?: string | null
  scrollY?: number
  text?: string | null
  devicePixelRatio?: number
  inner?: [number, number]
  samples?: Array<{
    pair: string
    first: { ink: number; signature: number }
    second: { ink: number; signature: number }
  }>
}

export interface LiveSite {
  /** The page the browser opens. */
  url: string
  /** The page that draws Chinese, Japanese and Korean text. */
  fontsUrl: string
  /** What the pages reported, oldest first. */
  records: LiveRecord[]
  /** The records of one kind, newest last. */
  of: (type: string) => LiveRecord[]
  stop: () => void
}

export interface LiveSiteOptions {
  /** The host a guest dials; the loopback address by default. */
  host?: string
  /** A file served at `/artifact`, for a guest that seeds its own commands. */
  artifact?: string
  /**
   * How often the page moves its mark. A Host with two processors composites
   * and encodes in software, so a page that moves sixty times a second leaves
   * it nothing for its own commands.
   */
  paintMs?: number
}

/** The fixture website, on a port the system picks. */
export function liveSite(options: LiveSiteOptions = {}): LiveSite {
  const records: LiveRecord[] = []
  const host = options.host ?? '127.0.0.1'
  const server = Bun.serve({
    port: 0,
    hostname: options.host ? '0.0.0.0' : '127.0.0.1',
    async fetch(request) {
      const { pathname } = new URL(request.url)
      if (pathname === '/record') {
        records.push(JSON.parse(await request.text()) as LiveRecord)
        return new Response(null, { status: 204 })
      }
      if (pathname === '/artifact' && options.artifact) {
        return new Response(Bun.file(options.artifact))
      }
      const body = pathname === '/fonts' ? FONTS_PAGE : page(options.paintMs ?? 16)
      return new Response(body, { headers: { 'content-type': 'text/html; charset=utf-8' } })
    },
  })
  const origin = `http://${host}:${server.port}`
  return {
    url: `${origin}/page`,
    fontsUrl: `${origin}/fonts`,
    records,
    of: (type) => records.filter((record) => record.type === type),
    stop: () => void server.stop(true),
  }
}

/** A message of the module's, as JSON carries it. */
export interface LiveMessage {
  type: string
  [key: string]: unknown
}

export interface LiveFrame {
  tab: string
  generation: number
  sequence: number
  key: boolean
  width: number
  height: number
  data: Uint8Array
}

const encoder = new TextEncoder()
const decoder = new TextDecoder()

/** The page's side of the live protocol over the conversation's stream. */
export class LiveView {
  private readonly socket: WebSocket
  private pending = new Uint8Array(0)
  readonly messages: LiveMessage[] = []
  readonly frames: LiveFrame[] = []
  /** Heartbeats are not messages the view acts on; a stall is their absence. */
  heartbeats = 0
  closed: { code: number; reason: string } | null = null
  /** Frames are acknowledged as a page that shows them does. */
  acknowledge = true

  constructor(world: { url: string; cookie: string }, conversation: string) {
    this.socket = new WebSocket(`${world.url.replace(/^http/, 'ws')}/api/conversations/${conversation}/streams/browser`, {
      headers: { cookie: world.cookie, origin: world.url },
    })
    this.socket.binaryType = 'arraybuffer'
    this.socket.addEventListener('message', (event) => this.read(new Uint8Array(event.data as ArrayBuffer)))
    this.socket.addEventListener('close', (event) => {
      this.closed = { code: event.code, reason: event.reason }
    })
  }

  async open(): Promise<void> {
    await new Promise((resolve) => this.socket.addEventListener('open', resolve))
    this.send({ type: 'hello', platform: 'mac' })
  }

  send(message: Record<string, unknown>): void {
    const json = encoder.encode(JSON.stringify(message))
    const frame = new Uint8Array(5 + json.length)
    new DataView(frame.buffer).setUint32(0, 1 + json.length)
    frame[4] = 1
    frame.set(json, 5)
    this.socket.send(frame)
  }

  /** A pointer of the viewer's, as the view sends one. */
  pointer(tab: string, action: string, x: number, y: number): void {
    this.send({
      type: 'pointer', tab, action, x, y,
      button: action === 'move' ? 'none' : 'left',
      buttons: action === 'down' ? 1 : 0,
      clickCount: action === 'move' ? 0 : 1, modifiers: 0,
    })
  }

  /** A key of the viewer's, pressed and released. */
  key(tab: string, value: string, code: string, keyCode: number, text?: string): void {
    for (const action of ['down', 'up']) {
      this.send({
        type: 'key', tab, action, key: value, code, keyCode, modifiers: 0,
        repeat: false, location: 0, altGraph: false,
        ...(action === 'down' && text ? { text } : {}),
      })
    }
  }

  private read(chunk: Uint8Array): void {
    const joined = new Uint8Array(this.pending.length + chunk.length)
    joined.set(this.pending)
    joined.set(chunk, this.pending.length)
    this.pending = joined
    for (;;) {
      if (this.pending.length < 4) {
        return
      }
      const view = new DataView(this.pending.buffer, this.pending.byteOffset)
      const length = view.getUint32(0)
      if (this.pending.length < 4 + length) {
        return
      }
      const payload = this.pending.subarray(5, 4 + length)
      const kind = this.pending[4]
      this.pending = this.pending.subarray(4 + length)
      if (kind === 1) {
        const message = JSON.parse(decoder.decode(payload)) as { type: string }
        if (message.type === 'heartbeat') {
          this.heartbeats += 1
        } else {
          this.messages.push(message as LiveMessage)
        }
      } else if (kind === 2) {
        const header = new DataView(payload.buffer, payload.byteOffset, 48)
        const frame = {
          tab: decoder.decode(payload.subarray(0, 24)).replace(/\0+$/, ''),
          generation: header.getUint32(24),
          sequence: header.getUint32(28),
          key: (header.getUint8(32) & 1) === 1,
          width: header.getUint16(44),
          height: header.getUint16(46),
          data: payload.subarray(48),
        }
        this.frames.push(frame)
        if (this.acknowledge) {
          this.send({ type: 'ack', generation: frame.generation, sequence: frame.sequence, decodeQueue: 0 })
        }
      }
    }
  }

  /** The newest message of `type`, if one arrived. */
  last(type: string): LiveMessage | undefined {
    for (let at = this.messages.length - 1; at >= 0; at -= 1) {
      if (this.messages[at]!.type === type) {
        return this.messages[at]
      }
    }
    return undefined
  }

  /** The next message of `type` after the ones already seen. */
  async message(type: string, since = 0): Promise<LiveMessage> {
    let found: LiveMessage | undefined
    await waitFor(
      () => {
        found = this.messages.slice(since).find((message) => message.type === type)
        return Boolean(found)
      },
      () => `no ${type}; saw ${this.messages
        .map((message) => message.type === 'notice'
          ? `notice(${String(message.code)}: ${String(message.message)})`
          : message.type)
        .join(', ')}`,
      { timeoutMs: 30_000 },
    )
    return found!
  }

  close(): void {
    this.socket.close()
  }
}

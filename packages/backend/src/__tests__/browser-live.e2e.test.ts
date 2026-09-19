import { afterAll, beforeAll, expect, test } from 'bun:test'
import { nativePackageFixture } from '@demicodes/host-remote/testing'
import { waitFor } from '@demicodes/utils'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'

/**
 * The live browser view end to end (`browser-live-view.md` § Acceptance): a
 * page's stream through the backend and the runner to the conversation's own
 * Chrome, against fixture pages that record what they received. Run it with
 * `DEMI_BROWSER_LIVE_E2E=1`; it downloads the pinned Chrome on first use.
 */
const acceptance = process.env.DEMI_BROWSER_LIVE_E2E === '1' ? test : test.skip

/** Stripes half a CSS pixel wide: flat grey at ratio 1, black and white above. */
const PAGE = `<!doctype html>
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
setInterval(() => { x = x > 300 ? 0 : x + 6; spin.style.left = x + 'px' }, 16)
</script>`

interface Record {
  type: string
  key: string | null
  target: string
  value: string | null
  scrollY: number
  text: string | null
  devicePixelRatio?: number
  inner?: [number, number]
}

let world: World
let site: { url: string; records: Record[]; stop: () => void }

function fixtureServer() {
  const records: Record[] = []
  const server = Bun.serve({
    port: 0,
    async fetch(request) {
      const { pathname } = new URL(request.url)
      if (pathname === '/record') {
        records.push(JSON.parse(await request.text()) as Record)
        return new Response(null, { status: 204 })
      }
      return new Response(PAGE, { headers: { 'content-type': 'text/html; charset=utf-8' } })
    },
  })
  return {
    url: `http://127.0.0.1:${server.port}/page`,
    records,
    stop: () => void server.stop(true),
  }
}

beforeAll(async () => {
  const builtin = await nativePackageFixture()
  site = fixtureServer()
  world = await World.create({ runners: ['alpha'], nativeCommands: builtin })
}, 180_000)

afterAll(async () => {
  site?.stop()
  await world?.close()
}, 60_000)

const encoder = new TextEncoder()
const decoder = new TextDecoder()

type Message = Record & { [key: string]: unknown }

/** The page's side of the live protocol over the conversation's stream. */
class View {
  private readonly socket: WebSocket
  private pending = new Uint8Array(0)
  readonly messages: Array<{ type: string } & Record<string, unknown>> = []
  readonly frames: Array<{ tab: string; generation: number; sequence: number; key: boolean; width: number; height: number; data: Uint8Array }> = []
  closed: { code: number; reason: string } | null = null
  /** Frames are acknowledged as a page that shows them does. */
  acknowledge = true

  constructor(id: string) {
    this.socket = new WebSocket(`${world.url.replace(/^http/, 'ws')}/api/conversations/${id}/streams/browser`, {
      headers: { cookie: world.backend.session.cookie, origin: world.url },
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
        if (message.type !== 'heartbeat') {
          this.messages.push(message as { type: string } & Record<string, unknown>)
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

  /** The next message of `type` after the ones already seen. */
  async message(type: string, since = 0): Promise<Record<string, unknown>> {
    let found: Record<string, unknown> | undefined
    await waitFor(
      () => {
        found = this.messages.slice(since).find((message) => message.type === type)
        return Boolean(found)
      },
      () => `no ${type}; saw ${this.messages.map((message) => message.type).join(', ')}`,
      { timeoutMs: 30_000 },
    )
    return found!
  }

  close(): void {
    this.socket.close()
  }
}

/** What the fixture pages recorded, newest last. */
function records(type: string): Record[] {
  return site.records.filter((record) => record.type === type)
}

acceptance('a viewer watches the conversation\'s browser, types into it and ends it', async () => {
  const driver = await world.conversation('runner:alpha')
  const opened = await driver.turn({
    model: [
      model.shell('open', `demi browser open '${site.url}' --json`, 120_000),
      model.say('opened'),
    ],
  })
  expect(opened.received[0]).toContain('"tab"')
  await waitFor(() => records('ready').length > 0, () => 'the page never loaded', { timeoutMs: 60_000 })

  const view = new View(driver.id)
  await view.open()
  view.send({
    type: 'panel', width: 800, height: 600, devicePixelRatio: 2,
    screenWidth: 1440, screenHeight: 900,
  })
  const state = await view.message('state')
  expect(state.running).toBe(true)
  const tabs = state.tabs as Array<{ id: string; url: string; createdBy: { kind: string } }>
  expect(tabs).toHaveLength(1)
  expect(tabs[0]!.createdBy.kind).toBe('agent')
  const tab = tabs[0]!.id

  // 1. Pictures: watching a tab starts a stream at the panel's size, with a
  // key frame first, and a moving page keeps sending.
  view.send({ type: 'watch', tab })
  await waitFor(() => view.messages.some((message) => message.type === 'stream' && message.width === 1600),
    () => 'no stream at the panel size', { timeoutMs: 30_000 })
  const stream = view.messages.findLast((message) => message.type === 'stream')!
  expect(stream.height).toBe(1200)
  const generation = stream.generation as number
  await waitFor(() => view.frames.some((frame) => frame.generation === generation),
    () => 'no picture', { timeoutMs: 30_000 })
  const first = view.frames.find((frame) => frame.generation === generation)!
  expect(first.key).toBe(true)
  expect([first.width, first.height]).toEqual([1600, 1200])
  expect([...first.data.subarray(0, 4)]).toEqual([0, 0, 0, 1])
  const painted = view.frames.length
  await waitFor(() => view.frames.length > painted + 3, () => 'the moving page stopped', { timeoutMs: 20_000 })

  // 5. The page sees an ordinary browser at the viewer's ratio.
  await waitFor(() => records('ready').at(-1)?.devicePixelRatio === 2,
    () => `ratios: ${records('ready').map((record) => record.devicePixelRatio).join(',')}`,
    { timeoutMs: 20_000 })
  const ready = records('ready').at(-1)!
  expect(ready.inner).toEqual([800, 600])
  expect(ready).toMatchObject({ webdriver: false, headless: false, hover: true, finePointer: true })
  expect((ready as unknown as { scrollbar: number }).scrollbar).toBeGreaterThan(0)

  // 3. Input reaches the page as a local browser delivers it.
  const pointer = (action: string, x: number, y: number) => view.send({
    type: 'pointer', tab, action, x, y,
    button: action === 'move' ? 'none' : 'left',
    buttons: action === 'down' ? 1 : 0,
    clickCount: action === 'move' ? 0 : 1, modifiers: 0,
  })
  const key = (value: string, code: string, keyCode: number, text?: string) => {
    for (const action of ['down', 'up']) {
      view.send({
        type: 'key', tab, action, key: value, code, keyCode, modifiers: 0,
        repeat: false, location: 0, altGraph: false,
        ...(action === 'down' && text ? { text } : {}),
      })
    }
  }
  pointer('down', 100, 136)
  pointer('up', 100, 136)
  key('h', 'KeyH', 72, 'h')
  key('i', 'KeyI', 73, 'i')
  key('Enter', 'Enter', 13)
  await waitFor(() => records('input').some((record) => record.target === 'field' && record.value === 'hi'),
    () => `field records: ${JSON.stringify(records('input'))}`, { timeoutMs: 20_000 })
  expect(records('keypress').some((record) => record.key === 'h')).toBe(true)
  expect(records('mousedown').some((record) => record.target === 'field')).toBe(true)

  // A wheel turn scrolls the CSS distance the viewer turned.
  view.send({ type: 'wheel', tab, x: 400, y: 300, deltaX: 0, deltaY: 200, modifiers: 0 })
  await waitFor(() => site.records.some((record) => record.scrollY >= 200),
    () => `the page never scrolled; records: ${JSON.stringify(site.records.slice(-4))}`,
    { timeoutMs: 20_000 })

  // 4. The agent's commands and the viewer's input both take effect.
  const filled = await driver.turn({
    model: [
      model.shell('fill', `demi browser fill ${tab} --css '#area' --text agent --json`, 60_000),
      model.say('filled'),
    ],
  })
  expect(filled.received[0]).toContain('exitCode: 0')
  pointer('down', 100, 190)
  pointer('up', 100, 190)
  key('x', 'KeyX', 88, 'x')
  await waitFor(() => records('input').some((record) => record.target === 'area' && record.value === 'agentx'),
    () => `area records: ${JSON.stringify(records('input').filter((record) => record.target === 'area'))}`,
    { timeoutMs: 20_000 })

  // 7. Closing the last tab ends the browser, and the view says so.
  view.send({ type: 'close', tab })
  const ended = await view.message('ended')
  expect(ended.reason).toBe('browser_ended')
  await waitFor(() => view.closed !== null, () => 'the stream stayed open', { timeoutMs: 30_000 })
  expect(view.closed).toMatchObject({ code: 1000 })
}, 300_000)

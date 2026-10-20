import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The device panel's view on the page (`crates/plugin-mobile/page/src/index.ts`): the
// picture follows the panel's size at once, while the Host hears the size
// it shows the screen at once a drag of the panel's edge has settled, as it
// restarts its capture for a larger one; and the scroll wheel reaches the
// device as a turn in its points. The view is the mobile plugin's
// DeviceView over a live session whose stream the test plays, on a page of
// happy-dom; happy-dom lays nothing out, so the area's size is what the
// test's ResizeObserver reports, as a browser's layout would. A drag waits
// for the settle the view itself waits for, about 150 ms. The picture lies
// under the device's frame, whose screen is a hole it shows through:
// bounded by the device's silhouette under a drawn frame's image, and
// halfway into the bezel under the frame the view draws.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })

/** Each observer the view made, which the test tells the area's size as a browser's layout would. */
const observers: Array<{ report(width: number, height: number): void }> = []
window.ResizeObserver = class {
  private target: Element | null = null
  constructor(private readonly callback: ResizeObserverCallback) {
    observers.push(this)
  }
  observe(target: Element): void {
    this.target = target
  }
  report(width: number, height: number): void {
    if (!this.target) {
      return
    }
    const box = { inlineSize: width, blockSize: height }
    this.callback([{
      target: this.target,
      contentRect: new DOMRectReadOnly(0, 0, width, height),
      contentBoxSize: [box],
      borderBoxSize: [box],
      devicePixelContentBoxSize: [box],
    }], this)
  }
  unobserve(): void {}
  disconnect(): void {
    this.target = null
  }
}

await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { LIVE_CONTROL_FRAME, SQUARE_CORNERS, encodeVideo } = await import('@demicodes/plugin-sdk')
const { drawnFrameBezel } = await import('@demicodes/web-ui/live/drawn-frame')
const { default: DeviceView } = await import('@demicodes/plugin-mobile/live/DeviceView.vue')
const { MobileSession } = await import('@demicodes/plugin-mobile/live/session')
type LiveModuleMessage = import('@demicodes/plugin-mobile/generated/plugin').LiveModuleMessage
type LiveViewerMessage = import('@demicodes/plugin-mobile/generated/plugin').LiveViewerMessage
type LiveFrame = import('@demicodes/plugin-mobile/generated/plugin').LiveFrame
type UserStreamHandlers = import('@demicodes/plugin-sdk').UserStreamHandlers

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  for (const name of DECODING) {
    Reflect.deleteProperty(globalThis, name)
  }
  observers.length = 0
  document.body.innerHTML = ''
})

/** The globals a picture is decoded and painted with, which happy-dom lacks. */
const DECODING = ['VideoDecoder', 'EncodedVideoChunk', 'requestAnimationFrame'] as const

/**
 * WebCodecs as the view uses it: each chunk decodes at once to a picture of
 * the size `size` gives then, painted at the next animation frame, at once.
 */
function stubDecoding(size: () => { width: number; height: number }): void {
  class Chunk {
    readonly timestamp: number
    constructor(init: EncodedVideoChunkInit) {
      this.timestamp = Math.trunc(init.timestamp)
    }
  }
  class Decoder {
    state: CodecState = 'unconfigured'
    constructor(private readonly init: VideoDecoderInit) {}
    configure(): void {
      this.state = 'configured'
    }
    decode(chunk: Chunk): void {
      const { width, height } = size()
      // The fields of a VideoFrame the view reads; happy-dom has no VideoFrame to build.
      this.init.output({ timestamp: chunk.timestamp, displayWidth: width, displayHeight: height, close() {} } as unknown as VideoFrame)
    }
    close(): void {
      this.state = 'closed'
    }
  }
  const values = { VideoDecoder: Decoder, EncodedVideoChunk: Chunk, requestAnimationFrame: (paint: () => void) => (paint(), 1) }
  for (const name of DECODING) {
    Object.defineProperty(globalThis, name, { configurable: true, value: values[name] })
  }
}

function framed(message: LiveModuleMessage): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(message))
  const bytes = new Uint8Array(5 + json.length)
  new DataView(bytes.buffer).setUint32(0, 1 + json.length)
  bytes[4] = LIVE_CONTROL_FRAME
  bytes.set(json, 5)
  return bytes
}

/** The view of device m1, a screen of 400 × 800 points in `frame`, in an area the test sizes; what it sends goes to `sent`. */
async function viewing(frame: LiveFrame = { kind: 'drawn', corners: SQUARE_CORNERS, buttons: [] }) {
  const sent: LiveViewerMessage[] = []
  let heard: (() => void) | null = null
  const streams: UserStreamHandlers[] = []
  const session = new MobileSession({
    open: (handlers) => {
      streams.push(handlers)
      return {
        send: (bytes) => {
          // The view's other frames acknowledge pictures.
          if (bytes[4] !== LIVE_CONTROL_FRAME) {
            return
          }
          sent.push(JSON.parse(new TextDecoder().decode(bytes.subarray(5))) as LiveViewerMessage)
          heard?.()
        },
        watched: () => {},
        close: () => {},
      }
    },
    platform: 'mac',
    defect: (_, error) => { throw error },
  })
  session.enable()
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(DeviceView, { session, device: 'm1', shown: true }) })
  app.mount(root)
  unmount = () => {
    app.unmount()
    session.close()
  }
  await nextTick()
  const area = (width: number, height: number) => {
    for (const observer of observers) {
      observer.report(width, height)
    }
    return nextTick()
  }
  await area(300, 500)
  streams[0]!.data(framed({
    type: 'state',
    devices: [{ id: 'm1', name: 'iPhone 17', platform: 'ios', os: 'iOS 26.5', model: 'iPhone 17', recording: false, orientation: 'portrait' }],
    watched: 'm1',
  }))
  streams[0]!.data(framed({
    type: 'frame',
    device: 'm1',
    width: 300,
    height: 500,
    scale: 1,
    orientation: 'portrait',
    pointsWidth: 400,
    pointsHeight: 800,
    frame,
  }))
  await nextTick()
  const canvas = root.querySelector('canvas')!
  return {
    sent,
    area,
    canvas,
    stream: streams[0]!,
    shown: () => sent.filter((message) => message.type === 'shown'),
    /** The next message the view sends of `type`. */
    next: (type: LiveViewerMessage['type']) => new Promise<void>((resolve) => {
      const before = sent.filter((message) => message.type === type).length
      heard = () => {
        if (sent.filter((message) => message.type === type).length > before) {
          heard = null
          resolve()
        }
      }
    }),
  }
}

test('a drag of the panel’s edge moves the picture at once and tells the Host the settled size once', async () => {
  const view = await viewing()
  if (view.shown().length === 0) {
    await view.next('shown')
  }
  const style = () => ({ width: parseFloat(view.canvas.style.width), height: parseFloat(view.canvas.style.height) })
  const first = style()
  expect(view.shown()).toEqual([{ type: 'shown', device: 'm1', ...first }])

  // The drag: the picture grows with each step; the Host hears nothing yet.
  const settled = view.next('shown')
  for (const height of [550, 600, 650, 700]) {
    await view.area(300 + (height - 500) / 2, height)
    expect(style().height).toBeGreaterThan(first.height)
  }
  expect(view.shown()).toHaveLength(1)
  const last = style()
  await settled
  expect(view.shown()).toEqual([
    { type: 'shown', device: 'm1', ...first },
    { type: 'shown', device: 'm1', ...last },
  ])
})

test('the scroll wheel turns the device’s screen by as far in its points', async () => {
  const view = await viewing()
  const { width, height, left, top } = {
    width: parseFloat(view.canvas.style.width),
    height: parseFloat(view.canvas.style.height),
    left: parseFloat(view.canvas.style.left),
    top: parseFloat(view.canvas.style.top),
  }
  // The picture's width and height are each whole pixels: each axis maps by its own.
  const scale = 400 / width
  const scaleY = 800 / height
  const turn = new WheelEvent('wheel', { deltaY: 120, deltaMode: 0, bubbles: true, cancelable: true })
  // happy-dom's WheelEvent has none of a mouse event's fields: given as a browser's would have them.
  Object.defineProperties(turn, {
    clientX: { value: left + 100 },
    clientY: { value: top + 200 },
    altKey: { value: false },
    ctrlKey: { value: false },
    metaKey: { value: false },
    shiftKey: { value: false },
  })
  view.canvas.dispatchEvent(turn)
  expect(turn.defaultPrevented).toBe(true)
  const turns = view.sent.flatMap((message) => message.type === 'input' && message.input.type === 'wheel' ? [message.input] : [])
  expect(turns).toEqual([{ type: 'wheel', x: 100 * scale, y: 200 * scaleY, deltaX: 0, deltaY: 120 * scale, modifiers: 0 }])
})

/** The points a path passes through, arcs' ends included. */
function pathPoints(path: string): { x: number; y: number }[] {
  const points: { x: number; y: number }[] = []
  for (const [, command, numbers] of path.matchAll(/([MLA])([^MLAZ]*)/g)) {
    const values = numbers!.trim().split(/\s+/).map(Number)
    const [x, y] = command === 'A' ? values.slice(5) : values
    points.push({ x: x!, y: y! })
  }
  return points
}

test('under a drawn frame’s image the picture is bounded by the device’s silhouette, not cut to the screen', async () => {
  const view = await viewing({
    kind: 'image',
    png: 'UE5H',
    silhouette: 'U0lM',
    width: 440,
    height: 860,
    screen: { x: 20, y: 30, width: 400, height: 800 },
  })
  const style = view.canvas.style
  const scale = parseFloat(style.width) / 400
  expect(style.clipPath).toBe('')
  expect(style.getPropertyValue('mask-image')).toBe('url(data:image/png;base64,U0lM)')
  // The silhouette lies over the whole frame, which starts 20 by 30 of its pixels before the screen.
  const pixels = (value: string) => value.split(' ').map(parseFloat)
  const [width, height] = pixels(style.getPropertyValue('mask-size'))
  const [left, top] = pixels(style.getPropertyValue('mask-position'))
  expect(width).toBeCloseTo(440 * scale, 3)
  expect(height).toBeCloseTo(860 * scale, 3)
  expect(left).toBeCloseTo(-20 * scale, 3)
  expect(top).toBeCloseTo(-30 * scale, 3)
})

test('under the frame the view draws the picture reaches halfway into its bezel', async () => {
  const round = { kind: 'round' as const, radius: 60 }
  const corners = { topLeft: round, topRight: round, bottomRight: round, bottomLeft: round }
  const view = await viewing({ kind: 'drawn', corners, buttons: [] })
  const width = parseFloat(view.canvas.style.width)
  const height = parseFloat(view.canvas.style.height)
  const clip = view.canvas.style.clipPath.match(/path\('(.*)'\)/)![1]!
  const points = pathPoints(clip)
  const half = drawnFrameBezel({ width, height }) / 2
  // Half a bezel out from each of the screen's edges, its corners round.
  expect(Math.min(...points.map((point) => point.x))).toBeCloseTo(-half, 1)
  expect(Math.min(...points.map((point) => point.y))).toBeCloseTo(-half, 1)
  expect(Math.max(...points.map((point) => point.x))).toBeCloseTo(width + half, 1)
  expect(Math.max(...points.map((point) => point.y))).toBeCloseTo(height + half, 1)
  expect(clip).toContain('A')
})

/**
 * The device announces its pictures in a new orientation before the first of
 * them is decoded: the canvas still shows the old picture, so it keeps that
 * picture's shape, turned toward the new orientation, until the new picture
 * is painted.
 *
 * Planted defect this catches: the view laying out the announced pictures
 * at once, which stretches the old picture into the new shape.
 */
test('a picture keeps its shape, turned, until the device’s pictures in their new orientation are painted', async () => {
  let size = { width: 800, height: 1600 }
  stubDecoding(() => size)
  const view = await viewing()
  const announce = (generation: number, orientation: 'portrait' | 'landscape-right', points: { width: number; height: number }) => {
    view.stream.data(framed({
      type: 'stream', device: 'm1', generation, width: size.width, height: size.height,
      pointsWidth: points.width, pointsHeight: points.height, orientation, scale: 1,
    }))
  }
  const picture = (generation: number) => {
    view.stream.data(encodeVideo({
      source: 'm1', generation, sequence: generation, key: true, timestamp: generation * 1000,
      width: size.width, height: size.height, data: new Uint8Array([0, 0, 0, 1, 0x65]),
    }))
  }
  announce(1, 'portrait', { width: 400, height: 800 })
  picture(1)
  await nextTick()
  // The placement is in whole pixels: the shape is close to the picture's, not exact.
  const shape = () => parseFloat(view.canvas.style.width) / parseFloat(view.canvas.style.height)
  const turned = () => view.canvas.parentElement!.style.transform
  expect(shape()).toBeCloseTo(0.5, 1)
  expect(turned()).toBe('')

  size = { width: 1600, height: 800 }
  announce(2, 'landscape-right', { width: 800, height: 400 })
  await nextTick()
  // Announced, not painted: the portrait picture stays portrait, turned a quarter.
  expect([view.canvas.width, view.canvas.height]).toEqual([800, 1600])
  expect(shape()).toBeCloseTo(0.5, 1)
  expect(turned()).toContain('rotate(90deg)')

  picture(2)
  await nextTick()
  expect([view.canvas.width, view.canvas.height]).toEqual([1600, 800])
  expect(shape()).toBeCloseTo(2, 1)
  expect(turned()).toBe('')
})

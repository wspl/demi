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
const { DeviceSession } = await import('@demicodes/plugin-mobile/live/session')
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
 * the size `size` gives then, painted at the next animation frame, which
 * comes once the current task's microtasks ran.
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
  // A browser paints at its next refresh, after the task that asked.
  const values = { VideoDecoder: Decoder, EncodedVideoChunk: Chunk, requestAnimationFrame: (paint: () => void) => (queueMicrotask(paint), 1) }
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

/**
 * Device `device`'s session, on a stream the test plays, whose sent
 * messages go to `sent`, its frames' images decoded by `decodeImage`, at
 * once unless the test says.
 */
function deviceSession(device: string, sent: LiveViewerMessage[], heard: () => void = () => {}, decodeImage = async () => {}) {
  const streams: UserStreamHandlers[] = []
  const session = new DeviceSession({
    device,
    model: 'iPhone 17',
    nativeId: null,
    decodeImage,
    open: (handlers) => {
      streams.push(handlers)
      return {
        send: (bytes) => {
          // The view's other frames acknowledge pictures.
          if (bytes[4] !== LIVE_CONTROL_FRAME) {
            return
          }
          sent.push(JSON.parse(new TextDecoder().decode(bytes.subarray(5))) as LiveViewerMessage)
          heard()
        },
        watched: () => {},
        close: () => {},
      }
    },
    platform: 'mac',
    defect: (_, error) => { throw error },
  })
  session.enable()
  return { session, stream: () => streams.at(-1)! }
}

/** Lets the frames' decoding under way complete and the view render. */
async function settled(): Promise<void> {
  for (let turn = 0; turn < 4; turn++) {
    await Promise.resolve()
  }
  await nextTick()
}

/** The view of device m1, a screen of 400 × 800 points in `frame`, in an area the test sizes; what it sends goes to `sent`. */
async function viewing(frame: LiveFrame = { kind: 'drawn', corners: SQUARE_CORNERS, buttons: [] }) {
  const sent: LiveViewerMessage[] = []
  let heard: (() => void) | null = null
  const { session, stream } = deviceSession('m1', sent, () => heard?.())
  const streams = [stream()]
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(DeviceView, { session, shown: true }) })
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
    device: { name: 'iPhone 17', platform: 'ios', os: 'iOS 26.5', model: 'iPhone 17', recording: false, orientation: 'portrait' },
  }))
  streams[0]!.data(framed({
    type: 'frame',
    width: 300,
    height: 500,
    scale: 1,
    orientation: 'portrait',
    pointsWidth: 400,
    pointsHeight: 800,
    frame,
  }))
  await settled()
  const canvas = root.querySelector('canvas')!
  return {
    session,
    sent,
    area,
    canvas,
    /** The device's screen, which the picture fills, under the frame. */
    screen: canvas.parentElement!,
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
  const style = () => ({ width: parseFloat(view.screen.style.width), height: parseFloat(view.screen.style.height) })
  const first = style()
  expect(view.shown()).toEqual([{ type: 'shown', ...first }])

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
    { type: 'shown', ...first },
    { type: 'shown', ...last },
  ])
})

test('the scroll wheel turns the device’s screen by as far in its points', async () => {
  const view = await viewing()
  const { width, height, left, top } = {
    width: parseFloat(view.screen.style.width),
    height: parseFloat(view.screen.style.height),
    left: parseFloat(view.screen.style.left),
    top: parseFloat(view.screen.style.top),
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
  const style = view.screen.style
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
  const width = parseFloat(view.screen.style.width)
  const height = parseFloat(view.screen.style.height)
  const clip = view.screen.style.clipPath.match(/path\('(.*)'\)/)![1]!
  const points = pathPoints(clip)
  const half = drawnFrameBezel({ width, height }) / 2
  // Half a bezel out from each of the screen's edges, its corners round.
  expect(Math.min(...points.map((point) => point.x))).toBeCloseTo(-half, 1)
  expect(Math.min(...points.map((point) => point.y))).toBeCloseTo(-half, 1)
  expect(Math.max(...points.map((point) => point.x))).toBeCloseTo(width + half, 1)
  expect(Math.max(...points.map((point) => point.y))).toBeCloseTo(height + half, 1)
  expect(clip).toContain('A')
})

/** The quarter turns, clockwise, of a `rotate` in `element`'s style. */
function quartersOf(element: Element | null): number {
  const turn = element instanceof HTMLElement ? element.style.transform.match(/rotate\((-?\d+)deg\)/) : null
  return turn ? Number(turn[1]) / 90 : 0
}

/** A box of the area, in CSS pixels. */
type Box = { left: number; top: number; width: number; height: number }

/**
 * Where `element` shows in an area of `area`: its box, turned with the device
 * by the quarter turns of the element that holds it, about the area's center.
 */
function shownBox(element: HTMLElement, area: { width: number; height: number }): Box {
  const box = { left: parseFloat(element.style.left), top: parseFloat(element.style.top), width: parseFloat(element.style.width), height: parseFloat(element.style.height) }
  let x = box.left + box.width / 2 - area.width / 2
  let y = box.top + box.height / 2 - area.height / 2
  let { width, height } = box
  for (let turn = 0; turn < ((quartersOf(element.parentElement) % 4) + 4) % 4; turn++) {
    [x, y, width, height] = [-y, x, height, width]
  }
  return { left: area.width / 2 + x - width / 2, top: area.height / 2 + y - height / 2, width, height }
}

/** A view whose device's pictures the test announces and paints, `size` pixels each. */
async function playing(frame?: LiveFrame) {
  let size = { width: 800, height: 1600 }
  stubDecoding(() => size)
  const view = await viewing(frame)
  let generation = 0
  const orientationPoints = (orientation: 'portrait' | 'landscape-right') => orientation === 'portrait'
    ? { width: 400, height: 800 }
    : { width: 800, height: 400 }
  return {
    ...view,
    /** The device announces its pictures in `orientation`. */
    announce: (orientation: 'portrait' | 'landscape-right') => {
      generation += 1
      const points = orientationPoints(orientation)
      size = { width: points.width * 2, height: points.height * 2 }
      view.stream.data(framed({
        type: 'stream', generation, width: size.width, height: size.height,
        pointsWidth: points.width, pointsHeight: points.height, orientation, scale: 1,
      }))
      return nextTick()
    },
    /** The first picture of the pictures announced last is decoded and painted. */
    picture: () => {
      view.stream.data(encodeVideo({
        source: 'm1', generation, sequence: generation, key: true, timestamp: generation * 1000,
        width: size.width, height: size.height, data: new Uint8Array([0, 0, 0, 1, 0x65]),
      }))
      return settled()
    },
    /** The Host draws the device's frame, `drawn`, for `orientation`. */
    frame: (orientation: 'portrait' | 'landscape-right', drawn: LiveFrame) => {
      const points = orientationPoints(orientation)
      view.stream.data(framed({
        type: 'frame', width: 300, height: 500, scale: 1, orientation,
        pointsWidth: points.width, pointsHeight: points.height, frame: drawn,
      }))
      return settled()
    },
    /** The quarter turns, clockwise, the canvas's picture shows turned by, with the device's and its own. */
    turned: () => quartersOf(view.screen.parentElement) + quartersOf(view.canvas),
  }
}

/**
 * The device announces its pictures in a new orientation before the first of
 * them is decoded: the canvas still shows the old picture, so it keeps that
 * picture's shape, turned toward the new orientation, until the new picture
 * is painted.
 *
 * Planted defects this catch: the view laying out the announced pictures
 * at once, which stretches the old picture into the new shape, and a new
 * picture painted before its frame.
 */
test('a picture keeps its shape, turned, until the device’s pictures in their new orientation are painted', async () => {
  const view = await playing()
  await view.announce('portrait')
  await view.picture()
  // The placement is in whole pixels: the shape is close to the picture's, not exact.
  const shape = () => parseFloat(view.canvas.style.width) / parseFloat(view.canvas.style.height)
  expect(shape()).toBeCloseTo(0.5, 1)
  expect(view.turned()).toBe(0)

  await view.announce('landscape-right')
  // Announced, not painted: the portrait picture stays portrait, turned a quarter.
  expect([view.canvas.width, view.canvas.height]).toEqual([800, 1600])
  expect(shape()).toBeCloseTo(0.5, 1)
  expect(view.turned()).toBe(1)

  // Painted only once its orientation's frame is ready, so the two show together.
  await view.picture()
  expect([view.canvas.width, view.canvas.height]).toEqual([800, 1600])
  expect(view.turned()).toBe(1)
  await view.frame('landscape-right', { kind: 'drawn', corners: SQUARE_CORNERS, buttons: [] })
  expect([view.canvas.width, view.canvas.height]).toEqual([1600, 800])
  expect(shape()).toBeCloseTo(2, 1)
  expect(view.turned()).toBe(0)
})

/** A frame's image of 440 × 860 pixels whose screen is 400 × 800 of them, and the same frame turned to landscape-right. */
const IMAGE_FRAMES = {
  'portrait': { kind: 'image', png: 'UE5H', silhouette: null, width: 440, height: 860, screen: { x: 20, y: 30, width: 400, height: 800 } },
  'landscape-right': { kind: 'image', png: 'TEFO', silhouette: null, width: 860, height: 440, screen: { x: 30, y: 20, width: 800, height: 400 } },
} as const satisfies Record<string, LiveFrame>
const ROUND = { kind: 'round' as const, radius: 60 }
const DRAWN_FRAME: LiveFrame = { kind: 'drawn', corners: { topLeft: ROUND, topRight: ROUND, bottomRight: ROUND, bottomLeft: ROUND }, buttons: [] }
const DRAWN_FRAMES = { 'portrait': DRAWN_FRAME, 'landscape-right': DRAWN_FRAME }

/**
 * Where the device shows: its screen and, for a frame's image, the image,
 * each as it shows in an area of `area`; a frame the view draws lies around
 * the screen, so its screen places it.
 */
function deviceBoxes(view: Awaited<ReturnType<typeof playing>>, area: { width: number; height: number }): { screen: Box; image: Box | null } {
  const image = view.screen.parentElement!.querySelector('img')
  return { screen: shownBox(view.screen, area), image: image ? shownBox(image, area) : null }
}

function expectSameBoxes(actual: { screen: Box; image: Box | null }, expected: { screen: Box; image: Box | null }): void {
  const close = (box: Box | null) => box && Object.fromEntries(Object.entries(box).map(([side, value]) => [side, Math.round(value * 1000) / 1000]))
  expect({ screen: close(actual.screen), image: close(actual.image) }).toEqual({ screen: close(expected.screen), image: close(expected.image) })
}

/**
 * A press of Rotate turns the device at once, laid out as the device's
 * pictures in the new orientation will be, so their first paint leaves the
 * device where it is, at a wide and a narrow panel's size, turning portrait,
 * landscape and back three times.
 *
 * Planted defect this catches: turning the device as it was laid out, its
 * scale capped at 1, which shows a portrait turned back from landscape at
 * the small landscape's size until its first picture paints.
 */
test.each([
  ['a frame’s image', 'wide', IMAGE_FRAMES, { width: 473, height: 916 }],
  ['a frame’s image', 'narrow', IMAGE_FRAMES, { width: 600, height: 816 }],
  ['the frame the view draws', 'wide', DRAWN_FRAMES, { width: 473, height: 916 }],
  ['the frame the view draws', 'narrow', DRAWN_FRAMES, { width: 600, height: 816 }],
] as const)('a turned device shows in %s where its new pictures paint, in a %s panel', async (_, _width, frames, area) => {
  const view = await playing(frames.portrait)
  await view.area(area.width, area.height)
  await view.announce('portrait')
  await view.picture()
  const portrait = deviceBoxes(view, area)
  for (let cycle = 0; cycle < 3; cycle++) {
    for (const [turn, orientation] of [['right', 'landscape-right'], ['left', 'portrait']] as const) {
      view.session.turning(turn)
      await nextTick()
      const turned = deviceBoxes(view, area)
      expect(view.turned()).toBe(turn === 'right' ? 1 : -1)
      // The device draws its frame for the new orientation, then announces and paints its pictures.
      await view.frame(orientation, frames[orientation])
      expectSameBoxes(deviceBoxes(view, area), turned)
      await view.announce(orientation)
      expectSameBoxes(deviceBoxes(view, area), turned)
      await view.picture()
      expect(view.turned()).toBe(0)
      expectSameBoxes(deviceBoxes(view, area), turned)
      if (orientation === 'portrait') {
        expectSameBoxes(turned, portrait)
      }
    }
  }
})

/**
 * Quick presses of Rotate Right then Left: the device draws its landscape
 * frame and announces its landscape pictures, then draws its portrait frame
 * before the first landscape picture paints, as a distant server's pictures
 * come late. The device, turned back, shows portrait in its portrait frame
 * throughout, the late landscape picture turned into its screen.
 *
 * Planted defect this catches: keeping only the newest frame and the shown
 * picture's, which paints the late landscape picture with no frame at all,
 * square-cornered and resized.
 */
test('quick turns back and forth never show the device without its frame', async () => {
  const view = await playing(IMAGE_FRAMES.portrait)
  const area = { width: 473, height: 916 }
  await view.area(area.width, area.height)
  await view.announce('portrait')
  await view.picture()
  const framedImage = () => view.screen.parentElement!.querySelector('img')?.getAttribute('src') ?? null
  const portrait = deviceBoxes(view, area)
  expect(framedImage()).toBe('data:image/png;base64,UE5H')
  view.session.turning('right')
  view.session.turning('left')
  await nextTick()
  const steps = [
    () => view.frame('landscape-right', IMAGE_FRAMES['landscape-right']),
    () => view.announce('landscape-right'),
    () => view.frame('portrait', IMAGE_FRAMES.portrait),
    () => view.picture(),
    () => view.announce('portrait'),
    () => view.picture(),
  ]
  for (const step of [() => Promise.resolve(), ...steps]) {
    await step()
    expect(framedImage()).toBe('data:image/png;base64,UE5H')
    expectSameBoxes(deviceBoxes(view, area), portrait)
  }
  expect(view.turned()).toBe(0)
})

const IPHONE_STATE = { name: 'iPhone 17', platform: 'ios', os: 'iOS 26.5', model: 'iPhone 17', recording: false, orientation: 'portrait' } as const

/** Announces a generation of 400 × 800 points on `stream` and sends its first picture, of `width` × `height` pixels as `size` decodes it. */
function deliver(stream: UserStreamHandlers, device: string, generation: number, width: number, height: number): void {
  stream.data(framed({ type: 'stream', generation, width, height, pointsWidth: 400, pointsHeight: 800, orientation: 'portrait', scale: 1 }))
  stream.data(encodeVideo({
    source: device, generation, sequence: generation, key: true, timestamp: generation * 1000,
    width, height, data: new Uint8Array([0, 0, 0, 1, 0x65]),
  }))
}

/**
 * The device shows its screen and its frame together or not at all: a
 * picture that comes before its frame waits, and so does a frame whose
 * image is not decoded yet.
 *
 * Planted defects this catches: a picture painted bare before its frame,
 * and a frame shown before its image can draw, which shows the screen
 * without its device around it.
 */
test('a device shows its frame and its picture together, never one without the other', async () => {
  let size = { width: 800, height: 1600 }
  stubDecoding(() => size)
  const sent: LiveViewerMessage[] = []
  const decoding: Array<() => void> = []
  const { session, stream } = deviceSession('m1', sent, () => {}, () => new Promise<void>((resolve) => {
    decoding.push(resolve)
  }))
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(DeviceView, { session, shown: true }) })
  app.mount(root)
  unmount = () => {
    app.unmount()
    session.close()
  }
  await nextTick()
  for (const observer of observers) {
    observer.report(300, 500)
  }
  await nextTick()
  stream().data(framed({ type: 'state', device: IPHONE_STATE }))
  deliver(stream(), 'm1', 1, 800, 1600)
  await settled()
  const showing = () => ({ device: root.querySelector('img') !== null, screen: session.canvas.isConnected && session.canvas.style.visibility !== 'hidden' })
  // The picture came first: nothing shows, the view says it waits.
  expect(showing()).toEqual({ device: false, screen: false })
  expect(root.textContent).toContain('frame')

  stream().data(framed({
    type: 'frame', width: 300, height: 500, scale: 1, orientation: 'portrait', pointsWidth: 400, pointsHeight: 800,
    frame: { kind: 'image', png: 'UE5H', silhouette: 'U0lM', width: 440, height: 860, screen: { x: 20, y: 30, width: 400, height: 800 } },
  }))
  await settled()
  // The frame's images, its own and the screen's shape, are not decoded yet: still nothing.
  expect(showing()).toEqual({ device: false, screen: false })
  expect(decoding).toHaveLength(2)
  decoding[0]!()
  await settled()
  expect(showing()).toEqual({ device: false, screen: false })
  decoding[1]!()
  await settled()
  expect(showing()).toEqual({ device: true, screen: true })
  expect([session.canvas.width, session.canvas.height]).toEqual([800, 1600])
})

/**
 * Two device tabs switched every few milliseconds while each device's late
 * pictures still arrive: each tab shows its own device's pictures only, a
 * late picture paints on its own device's canvas, and each device's
 * stream hears once of each change of whether it is looked at; with both
 * shown at once, as side by side, both stream.
 *
 * Planted defects this catches: a late picture of the device left a moment
 * ago painted into the new tab, which resizes its frame, and a switch that
 * tells a device more than once, or tells it of the other's.
 */
test('switching device tabs fast shows each tab its own device, however late the other’s pictures come', async () => {
  let size = { width: 800, height: 1600 }
  stubDecoding(() => size)
  const views = ['m1', 'm2'].map((device) => {
    const sent: LiveViewerMessage[] = []
    return { device, sent, ...deviceSession(device, sent) }
  })
  const { ref } = await import('vue')
  const shown = ref<string[]>(['m1'])
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => views.map((view) => h('div', { 'data-device': view.device }, [h(DeviceView, { session: view.session, shown: shown.value.includes(view.device) })])),
  })
  app.mount(root)
  unmount = () => {
    app.unmount()
    for (const view of views) {
      view.session.close()
    }
  }
  await nextTick()
  for (const observer of observers) {
    observer.report(300, 500)
  }
  await nextTick()
  // Each device's pictures have a size of their own: m1's 800 wide, m2's 600.
  const width = { m1: 800, m2: 600 } as const
  for (const view of views) {
    view.stream().data(framed({ type: 'state', device: IPHONE_STATE }))
    view.stream().data(framed({ type: 'frame', width: 300, height: 500, scale: 1, orientation: 'portrait', pointsWidth: 400, pointsHeight: 800, frame: { kind: 'drawn', corners: SQUARE_CORNERS, buttons: [] } }))
  }
  await settled()
  const generations = { m1: 0, m2: 0 }
  const picture = (device: 'm1' | 'm2') => {
    const view = views.find((each) => each.device === device)!
    generations[device] += 1
    size = { width: width[device], height: width[device] * 2 }
    deliver(view.stream(), device, generations[device], size.width, size.height)
  }
  const check = () => {
    for (const view of views) {
      const holder = root.querySelector(`[data-device="${view.device}"]`)!
      const canvases = [...holder.querySelectorAll('canvas')]
      // Each tab holds its own device's canvas, and that canvas only its device's pictures.
      expect(canvases.every((canvas) => canvas === view.session.canvas)).toBe(true)
      expect(view.session.canvas.width === 0 || view.session.canvas.width === width[view.device as 'm1' | 'm2'] || view.session.canvas.width === 300).toBe(true)
      expect(view.session.state.look?.picture?.width ?? width[view.device as 'm1' | 'm2']).toBe(width[view.device as 'm1' | 'm2'])
    }
  }
  picture('m1')
  await settled()
  check()
  for (let switchTo = 0; switchTo < 6; switchTo++) {
    const next = switchTo % 2 === 0 ? 'm2' : 'm1'
    const left = next === 'm2' ? 'm1' : 'm2'
    shown.value = [next]
    await nextTick()
    // The device left a moment ago still sends what was under way, then the new one its own.
    picture(left)
    picture(next)
    await settled()
    check()
  }
  const looks = (device: string) => views.find((view) => view.device === device)!.sent.flatMap((message) => message.type === 'looked' ? [message.looked] : [])
  expect(looks('m1')).toEqual([true, false, true, false, true, false, true])
  expect(looks('m2')).toEqual([true, false, true, false, true, false])

  // Side by side: both are looked at, and each streams on its own.
  shown.value = ['m1', 'm2']
  await nextTick()
  picture('m1')
  picture('m2')
  await settled()
  check()
  expect([looks('m1').at(-1), looks('m2').at(-1)]).toEqual([true, true])
  expect(views.map((view) => view.session.state.look?.picture?.width)).toEqual([800, 600])
})

import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The device panel's view on the page (`mobile.md` § The live view): the
// picture follows the panel's size at once, while the Host hears the size
// it shows the screen at once a drag of the panel's edge has settled, as it
// restarts its capture for a larger one; and the scroll wheel reaches the
// device as a turn in its points. The view is the mobile plugin's
// DeviceView over a live session whose stream the test plays, on a page of
// happy-dom; happy-dom lays nothing out, so the area's size is what the
// test's ResizeObserver reports, as a browser's layout would. A drag waits
// for the settle the view itself waits for, about 150 ms.

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
const { LIVE_CONTROL_FRAME } = await import('@demicodes/plugin-sdk')
const { default: DeviceView } = await import('@demicodes/plugin-mobile/live/DeviceView.vue')
const { MobileSession } = await import('@demicodes/plugin-mobile/live/session')
type LiveModuleMessage = import('@demicodes/plugin-mobile/generated/plugin').LiveModuleMessage
type LiveViewerMessage = import('@demicodes/plugin-mobile/generated/plugin').LiveViewerMessage
type UserStreamHandlers = import('@demicodes/plugin-sdk').UserStreamHandlers

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  observers.length = 0
  document.body.innerHTML = ''
})

function framed(message: LiveModuleMessage): Uint8Array {
  const json = new TextEncoder().encode(JSON.stringify(message))
  const bytes = new Uint8Array(5 + json.length)
  new DataView(bytes.buffer).setUint32(0, 1 + json.length)
  bytes[4] = LIVE_CONTROL_FRAME
  bytes.set(json, 5)
  return bytes
}

/** The view of device m1, a screen of 400 × 800 points, in an area the test sizes; what it sends goes to `sent`. */
async function viewing() {
  const sent: LiveViewerMessage[] = []
  let heard: (() => void) | null = null
  const streams: UserStreamHandlers[] = []
  const session = new MobileSession({
    open: (handlers) => {
      streams.push(handlers)
      return {
        send: (bytes) => {
          sent.push(JSON.parse(new TextDecoder().decode(bytes.subarray(5))) as LiveViewerMessage)
          heard?.()
        },
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
    devices: [{ id: 'm1', name: 'iPhone 17', platform: 'ios', os: 'iOS 26.5', recording: false }],
    watched: 'm1',
  }))
  streams[0]!.data(framed({
    type: 'frame',
    device: 'm1',
    orientation: 'portrait',
    pointsWidth: 400,
    pointsHeight: 800,
    frame: { kind: 'outline', cornerRadius: 40, buttons: [] },
  }))
  await nextTick()
  const canvas = root.querySelector('canvas')!
  return {
    sent,
    area,
    canvas,
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
  const { width, left, top } = { width: parseFloat(view.canvas.style.width), left: parseFloat(view.canvas.style.left), top: parseFloat(view.canvas.style.top) }
  const scale = 400 / width
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
  expect(turns).toEqual([{ type: 'wheel', x: 100 * scale, y: 200 * scale, deltaX: 0, deltaY: 120 * scale, modifiers: 0 }])
})

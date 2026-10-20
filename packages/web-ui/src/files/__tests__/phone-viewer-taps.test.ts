import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// On a phone the media viewer covers the screen, so a tap on the empty space
// beside the picture closes it, while a tap on the picture toggles its actual
// size and a drag or a swipe beside it does not close.
//
// The viewer is MediaViewer on a page of happy-dom 390 px wide with reduced
// motion, so a swipe steps at once. Happy-dom lays nothing out and loads no
// picture, so the picture's frame is given 358 × 600 px, its box at the page's
// origin, and the picture 1600 × 800 pixels, which fits at 358 × 179, 210.5 px
// from the frame's top.
//
// Cost: a few ms, besides the start of the page every happy-dom test pays.

GlobalRegistrator.register({
  url: 'https://demi.test/',
  width: 390,
  height: 844,
  settings: { device: { prefersReducedMotion: 'reduce' } },
})

const FRAME = { width: 358, height: 600 }

// The frame's size, as a browser's layout would report it once it is drawn.
window.ResizeObserver = class {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element): void {
    const box = { inlineSize: FRAME.width, blockSize: FRAME.height }
    const entry: ResizeObserverEntry = {
      target,
      contentRect: new DOMRectReadOnly(0, 0, FRAME.width, FRAME.height),
      contentBoxSize: [box],
      borderBoxSize: [box],
      devicePixelContentBoxSize: [box],
    }
    queueMicrotask(() => this.callback([entry], this))
  }
  unobserve(): void {}
  disconnect(): void {}
}

await import('../../__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { createMediaViewer } = await import('../media-viewer')
const { default: MediaViewer } = await import('../MediaViewer.vue')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const pictures = ['a', 'b', 'c'].map((name) => ({ kind: 'image' as const, src: `/blob/${name}`, name: `${name}.png`, size: { width: 1600, height: 800 } }))

/** Lets the page settle: the frame measured, the picture loaded and drawn. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 4; turn++) {
    await nextTick()
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

/** The viewer open on the phone at the middle picture of three; `frame` is where a finger touches the picture's area. */
async function openViewer() {
  const viewer = createMediaViewer(() => ({ page: { width: 390, height: 844 }, pixelRatio: 1 }))
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(() => h(MediaViewer, { viewer }))
  app.mount(host)
  unmount = () => app.unmount()
  viewer.show(pictures[1]!, () => pictures)
  await settle()
  const picture = () => {
    const image = document.querySelector<HTMLImageElement>('[role="dialog"] img')!
    if (!image.style.width) {
      Object.defineProperty(image, 'naturalWidth', { value: 1600 })
      Object.defineProperty(image, 'naturalHeight', { value: 800 })
      image.dispatchEvent(new Event('load'))
    }
    return image
  }
  picture()
  await settle()
  const frame = () => picture().parentElement!.parentElement!
  return { viewer, picture, frame }
}

/** A finger pressed at `from`, moved through `path` and lifted, on `target`. */
async function finger(target: Element, from: [number, number], ...path: [number, number][]): Promise<void> {
  const at = (type: string, [x, y]: [number, number]) => target.dispatchEvent(new PointerEvent(type, {
    bubbles: true, cancelable: true, pointerId: 1, pointerType: 'touch', button: type === 'pointerdown' ? 0 : -1, clientX: x, clientY: y,
  }))
  at('pointerdown', from)
  for (const point of path)
    at('pointermove', point)
  at('pointerup', path.at(-1) ?? from)
  await settle()
}

test('a tap beside the fitted picture closes the viewer', async () => {
  const { viewer, frame } = await openViewer()
  await finger(frame(), [179, 80])
  expect(viewer.shown.value).toBeNull()
})

test('a tap on the picture shows it at its actual size, and leaves the viewer open', async () => {
  const { viewer, picture, frame } = await openViewer()
  expect(picture().style.width).toBe('358px')
  await finger(frame(), [179, 300])
  expect(viewer.shown.value?.name).toBe('b.png')
  expect(picture().style.width).toBe('1600px')
})

test('a drag beside the picture moves nothing and closes nothing', async () => {
  const { viewer, frame } = await openViewer()
  await finger(frame(), [179, 80], [179, 110], [179, 160])
  expect(viewer.shown.value?.name).toBe('b.png')
})

test('a swipe that starts beside the picture steps to the next one and leaves the viewer open', async () => {
  const { viewer, frame } = await openViewer()
  await finger(frame(), [300, 80], [250, 80], [100, 80], [40, 80])
  expect(viewer.shown.value?.name).toBe('c.png')
})

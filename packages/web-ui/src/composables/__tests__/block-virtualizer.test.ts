import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The transcript's list draws the rows near the view as the view moves,
// within a row taller than the view too (`useBlockVirtualizer`). One Vue app
// on a happy-dom page, its boxes laid out by the test; a few ms besides the
// page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
const { createApp, defineComponent, h, nextTick, ref } = await import('vue')
const { useBlockVirtualizer } = await import('../useBlockVirtualizer')

/**
 * happy-dom lays nothing out: each box is as tall as its `data-height` says,
 * and nothing resizes.
 */
const rect = HTMLElement.prototype.getBoundingClientRect
HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
  const height = Number(this.dataset['height'] ?? 0)
  return { x: 0, y: 0, top: 0, left: 0, width: 600, height, right: 600, bottom: height, toJSON: () => ({}) }
}
const offsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')
const offsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth')
Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get(this: HTMLElement) { return Number(this.dataset['height'] ?? 0) } })
Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, get: () => 600 })
/**
 * Reports each box it is given once, at its size, as a browser's first
 * observation does.
 */
globalThis.ResizeObserver = class {
  constructor(private readonly report: ResizeObserverCallback) {}
  observe(target: Element): void {
    const height = target instanceof HTMLElement ? target.offsetHeight : 0
    queueMicrotask(() => this.report([{
      target,
      borderBoxSize: [{ blockSize: height, inlineSize: 600 }],
      contentBoxSize: [],
      devicePixelContentBoxSize: [],
      contentRect: target.getBoundingClientRect(),
    }], this))
  }
  unobserve(): void {}
  disconnect(): void {}
}

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/**
 * Lets the list render until its rows stop changing, as the measurements of the
 * rows it mounts land.
 */
async function settled(drawn: { value: number[] }): Promise<void> {
  for (let renders = 0; renders < 20; renders += 1) {
    const before = drawn.value.join()
    await nextTick()
    await new Promise((resolve) => setImmediate(resolve))
    if (drawn.value.join() === before)
      return
  }
  throw new Error(`The list did not settle: it draws ${drawn.value.join()}`)
}

test('scrolling within a row taller than the view brings the next row once it comes near', async () => {
  const drawn = ref<number[]>([])
  const root = document.createElement('div')
  document.body.append(root)
  let scroller: HTMLElement | undefined
  const app = createApp(defineComponent({
    setup() {
      const container = ref<HTMLElement>()
      const blocks = ref(Array.from({ length: 3 }, (_, index) => ({ id: `b${index}`, type: 'text' })))
      // The reader is at the top of the list.
      const list = useBlockVirtualizer(container, blocks, { anchor: { blockId: 'b0', anchorIndex: 0, offsetPx: 0, scrollTop: 0 }, heightCache: new Map() })
      return () => {
        drawn.value = list.virtualItems.value.map((item) => item.index)
        const scrollerRef = (el: unknown) => {
          container.value = el instanceof HTMLElement ? el : undefined
          scroller = container.value
        }
        const rowRef = (el: unknown) => list.measureElement(el instanceof Element ? el : null)
        return h('div', { ref: scrollerRef, 'data-height': '500', onScroll: list.onScroll }, [
          h('div', { style: { height: `${list.totalSize.value}px` } }, list.virtualItems.value.map((item) =>
            h('div', { key: item.index, 'data-index': item.index, 'data-height': '5000', ref: rowRef }))),
        ])
      }
    },
  }))
  app.mount(root)
  unmount = () => app.unmount()
  await settled(drawn)
  expect(drawn.value).toEqual([0])

  // 500 px before the first row's end: the next row lies within a view's height
  // below.
  scroller!.scrollTop = 4300
  scroller!.dispatchEvent(new Event('scroll'))
  await settled(drawn)
  expect(drawn.value).toEqual([0, 1])
})

afterEach(() => {
  HTMLElement.prototype.getBoundingClientRect = rect
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', offsetHeight!)
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', offsetWidth!)
})

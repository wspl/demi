import { afterEach, beforeEach, expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { startServiceWorker } from './page'

// The page's side of its service worker, on a web browser the test plays:
// its worker container, its document and the files it timed. Each test takes
// a few milliseconds.

const ORIGIN = 'https://demi.test'

/** A worker as the page holds it: what it was told. */
class TestWorker {
  readonly heard: unknown[] = []
  postMessage(message: unknown): void {
    this.heard.push(message)
  }
}

/** The page's worker container: `ready` names the registration's active worker, `controller` the one in control. */
class TestContainer extends EventTarget {
  controller: TestWorker | null
  readonly ready: Promise<{ active: TestWorker }>
  constructor(active: TestWorker) {
    super()
    this.controller = active
    this.ready = Promise.resolve({ active })
  }
  register(): Promise<void> {
    return Promise.resolve()
  }
  startMessages(): void {}
  /** A newer worker takes the page over. */
  takeOver(worker: TestWorker): void {
    this.controller = worker
    this.dispatchEvent(new Event('controllerchange'))
  }
}

const saved = ['navigator', 'document', 'location', 'window'].map((name) => [name, Reflect.get(globalThis, name)] as const)

beforeEach(() => {
  process.env.DEMI_WEB_BUILD = 'b2'
  Reflect.set(globalThis, 'document', {
    readyState: 'complete',
    querySelectorAll: (selector: string) => selector.startsWith('script') ? [{ src: `${ORIGIN}/assets/index-1a2b.js` }] : [],
  })
  Reflect.set(globalThis, 'location', { href: `${ORIGIN}/chat`, origin: ORIGIN })
  Reflect.set(globalThis, 'window', globalThis)
})

afterEach(() => {
  delete process.env.DEMI_WEB_BUILD
  for (const [name, value] of saved) {
    Reflect.set(globalThis, name, value)
  }
})

test('a worker that takes the page over after the one it announced to hears the page’s build too', async () => {
  const old = new TestWorker()
  const container = new TestContainer(old)
  Reflect.set(globalThis, 'navigator', { serviceWorker: container })
  startServiceWorker()
  await waitFor(() => old.heard.length === 1)
  // The first visit after an upgrade: the new worker activates and takes over.
  const upgraded = new TestWorker()
  container.takeOver(upgraded)
  expect(upgraded.heard).toEqual([{ type: 'demi-page', build: 'b2', files: ['/assets/index-1a2b.js'] }])
})

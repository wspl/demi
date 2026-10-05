import type { ProductState, SyncEvent } from '../api/generated/web-api'

/**
 * A page's synchronization channel in a test, which the test plays as the
 * backend would (`web-api.md` § Page synchronization): it opens the channel,
 * sends its messages and ends it.
 */
export class TestChannel {
  readonly url: string
  onopen: (() => void) | null = null
  onmessage: ((message: { data: string }) => void) | null = null
  onclose: ((close: { code: number; reason: string }) => void) | null = null
  /** Whether the page closed the channel itself. */
  closed = false

  constructor(url: string | URL) {
    this.url = String(url)
    opened.push(this)
  }

  /** The page closes it. */
  close(): void {
    this.closed = true
  }

  /** The upgrade succeeds. */
  open(): void {
    this.onopen?.()
  }

  send(event: SyncEvent): void {
    this.onmessage?.({ data: JSON.stringify(event) })
  }

  /** The channel opens and sends `state` first, as every connection does. */
  connect(state: ProductState): void {
    this.open()
    this.send({ type: 'snapshot', state })
  }

  /** The backend or the network ends it. */
  end(code = 1006, reason = ''): void {
    this.onclose?.({ code, reason })
  }
}

const opened: TestChannel[] = []

/**
 * Plays every channel a page opens from now on, until `restore`: the
 * channels in the order the page opened them, the newest last. A test runs
 * without a document, so the page's address is the product's.
 */
export function playChannels(): { readonly opened: readonly TestChannel[]; last(): TestChannel; restore(): void } {
  const realSocket = globalThis.WebSocket
  const realWindow = Reflect.get(globalThis, 'window')
  opened.length = 0
  // The page opens its channel with `new WebSocket(url)`, which the test
  // channel answers in the socket's place.
  Reflect.set(globalThis, 'WebSocket', TestChannel)
  if (realWindow === undefined) {
    Reflect.set(globalThis, 'window', { location: { href: 'http://127.0.0.1:3271/chat' } })
  }
  return {
    opened,
    last(): TestChannel {
      // A conversation's socket is no synchronization channel.
      const channel = opened.findLast((candidate) => new URL(candidate.url).pathname.endsWith('/sync'))
      if (!channel) {
        throw new Error('The page opened no synchronization channel')
      }
      return channel
    },
    restore(): void {
      Reflect.set(globalThis, 'WebSocket', realSocket)
      Reflect.set(globalThis, 'window', realWindow)
      opened.length = 0
    },
  }
}

import type { ProductState, SyncEvent } from '../api/generated/web-api'

/**
 * A page's synchronization channel in a test, which the test plays as the
 * backend would (`web-api.md` § Page synchronization): it opens the channel,
 * sends its messages and ends it.
 */
export class TestChannel extends EventTarget {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readonly url: string
  readyState: number = TestChannel.CONNECTING
  onopen: (() => void) | null = null
  onmessage: ((message: { data: string }) => void) | null = null
  onclose: ((close: { code: number; reason: string }) => void) | null = null
  /** Whether the page closed the channel itself. */
  closed = false

  constructor(url: string | URL) {
    super()
    this.url = String(url)
    opened.push(this)
  }

  /**
   * The page closes it. One still connecting fails, as the web browser
   * fails it, with a close; one open waits for the backend's answer to its
   * close, which never comes in a test.
   */
  close(): void {
    const connecting = this.readyState === TestChannel.CONNECTING
    this.closed = true
    this.readyState = TestChannel.CLOSED
    if (connecting) {
      this.closeWith(1006, '')
    }
  }

  /** The upgrade succeeds. */
  open(): void {
    this.readyState = TestChannel.OPEN
    this.onopen?.()
    this.dispatchEvent(new Event('open'))
  }

  send(event: SyncEvent): void {
    this.deliver(event)
  }

  /** A message as another build's backend may send it, outside this page's contract. */
  deliver(message: unknown): void {
    const data = JSON.stringify(message)
    this.onmessage?.({ data })
    this.dispatchEvent(new MessageEvent('message', { data }))
  }

  /** The channel opens and sends `state` first, as every connection does. */
  connect(state: ProductState): void {
    this.open()
    this.send({ type: 'snapshot', state })
  }

  /** The backend or the network ends it. */
  end(code = 1006, reason = ''): void {
    this.readyState = TestChannel.CLOSED
    this.closeWith(code, reason)
  }

  private closeWith(code: number, reason: string): void {
    this.onclose?.({ code, reason })
    this.dispatchEvent(new CloseEvent('close', { code, reason }))
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

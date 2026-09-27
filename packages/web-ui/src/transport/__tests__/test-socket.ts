import type { ClientFrame, ServerFrame } from '@demicodes/protocol'

/**
 * A conversation socket in a test, which the test plays as the backend
 * would: it lets the socket open, sends frames and ends it.
 */
export class TestSocket extends EventTarget {
  /** Whether the page closed the socket, or the test ended it. */
  closed = false
  /** What the page sent, as frames. */
  readonly sent: ClientFrame[] = []

  constructor(readonly url: string) {
    super()
    opened.push(this)
  }

  send(data: string): void {
    this.sent.push(JSON.parse(data))
  }

  /** The page closes it. */
  close(): void {
    if (this.closed) {
      return
    }
    this.closed = true
    this.dispatchEvent(new Event('close'))
  }

  /** The upgrade succeeds. */
  open(): void {
    this.dispatchEvent(new Event('open'))
  }

  receive(frame: ServerFrame): void {
    this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(frame) }))
  }

  /** The backend or the network ends it, as a backend that restarts does. */
  end(): void {
    this.closed = true
    this.dispatchEvent(new CloseEvent('close', { code: 1006 }))
  }
}

const opened: TestSocket[] = []

/**
 * Plays every socket the page opens from now on, until `restore`: the
 * sockets in the order the page opened them, the newest last.
 */
export function playSockets(): { last(): TestSocket; restore(): void } {
  const realSocket = globalThis.WebSocket
  opened.length = 0
  // The page opens its socket with `new WebSocket(url)`, which the test
  // socket answers in the browser socket's place.
  Reflect.set(globalThis, 'WebSocket', TestSocket)
  return {
    last(): TestSocket {
      const socket = opened.at(-1)
      if (!socket) {
        throw new Error('The page opened no socket')
      }
      return socket
    },
    restore(): void {
      Reflect.set(globalThis, 'WebSocket', realSocket)
      opened.length = 0
    },
  }
}

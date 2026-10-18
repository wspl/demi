import type { ClientFrame, PagePing, ServerFrame } from '@demicodes/protocol'

/**
 * A conversation socket in a test, which the test plays as the backend
 * would: it lets the socket open, sends frames and ends it.
 */
export class TestSocket extends EventTarget {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readyState: number = TestSocket.CONNECTING
  /** Whether the page closed the socket, or the test ended it. */
  closed = false
  /** What the page sent, as frames. */
  readonly sent: ClientFrame[] = []
  /** The ids of the pings the page sent as it returned. */
  readonly pings: number[] = []

  constructor(readonly url: string) {
    super()
    opened.push(this)
  }

  send(data: string): void {
    const message: ClientFrame | PagePing = JSON.parse(data)
    if ('ping' in message) {
      this.pings.push(message.ping)
    } else {
      this.sent.push(message)
    }
  }

  /** The page closes it. */
  close(): void {
    if (this.closed) {
      return
    }
    this.closed = true
    this.readyState = TestSocket.CLOSED
    this.dispatchEvent(new Event('close'))
  }

  /** The upgrade succeeds. */
  open(): void {
    this.readyState = TestSocket.OPEN
    this.dispatchEvent(new Event('open'))
  }

  /** One message: one frame as an object, several as an array, as the backend sends frames ready together. */
  receive(...frames: [ServerFrame, ...ServerFrame[]]): void {
    const message = frames.length === 1 ? frames[0] : frames
    this.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(message) }))
  }

  /** The backend or the network ends it, as a backend that restarts does. */
  end(): void {
    this.closed = true
    this.readyState = TestSocket.CLOSED
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
  // socket answers in place of the web browser's socket.
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

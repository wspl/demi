/**
 * The page connection: the one WebSocket a client holds to a core, and the
 * lanes it carries (`crates/web-api-protocol/src/connection.rs`).
 *
 * Each part of a client that talks with the core over a socket opens a lane:
 * the synchronization channel, each conversation's lane, the file watches, a
 * device's signaling, the user streams and a program's plugins. A lane looks
 * to its owner as a WebSocket does: it opens or fails, carries messages, and
 * closes with a code and a reason; a lane the core refused says why.
 *
 * ```text
 *  owners --- openLane ---> lanes --- one queue, in turn ---> WebSocket
 *     ^                                                          |
 *     `---------- open, message, close events <---- frames ------'
 * ```
 *
 * - The connection opens as the first lane is asked for, once it started,
 *   and connects again after it is lost, while a lane wants it, after the
 *   wait its host gives. As it opens again, its host hears it, as a page
 *   connects every owner that waits at once.
 * - Its liveness is its host's (`ConnectionHost.watch`); a lane has none of
 *   its own.
 * - A lane asked for while the connection is down waits for it, and its
 *   open's 10 seconds start as the open goes. A lost connection closes each
 *   open lane with the connection's close, 1006 when it broke, and sends each
 *   lane still opening again on the next connection.
 * - The client writes the connection's own messages and the lanes' opens and
 *   closes first, then one piece of each lane's message in turn, a user
 *   stream's after the others', and a piece only while the socket holds less
 *   than 64 KiB unsent. A lane starts no more of its messages than the core's
 *   credit allows.
 * - A lane's messages follow its open without waiting for `opened`; a lane
 *   still opening when the connection is lost sends them again behind its
 *   open on the next.
 *
 * Where it runs is its host's: a web page opens a browser's WebSocket and
 * watches the page's returns (`web-ui`'s `transport/page-connection.ts`); a
 * program's SDK frames the socket on an HTTP/2 stream to its process.
 */
import {
  BINARY_FLAG,
  CLOSE_FRAME,
  CONNECTION_FRAME,
  CREDIT_FRAME,
  DATA_FRAME,
  LANE_CREDIT,
  LANE_PIECE_BYTES,
  MORE_FLAG,
  OPENED_FRAME,
  OPEN_FRAME,
  REFUSED_FRAME,
  backendConnectionMessageSchema,
  laneRefusalSchema,
  type LaneRefusal,
  type LaneTarget,
  type PageConnectionMessage,
} from '@demicodes/protocol'

export type { LaneRefusal, LaneTarget }

/** How long a lane's open may wait for its answer once it went. */
const OPEN_MS = 10_000
/** How much the socket may hold unsent before a lane's next piece waits. */
const BUFFERED_BYTES = 64 * 1024
/** How long a piece waits for the socket to send what it holds before the client looks again. */
const DRAIN_MS = 16
/**
 * How long an open connection that brought nothing must stay open before it
 * counts as connected: a core that accepts the upgrade and closes it at once
 * is not retried at the rate of a quiet network.
 */
const ESTABLISHED_MS = 1_000
/** The close of a connection whose server shuts down, and will be back. */
const BACKEND_CLOSING = 1001
/** The close a lane hears of a connection that ended without a close, or of an open that failed. */
const ABNORMAL_CLOSURE = 1006

/** How a socket hands over a binary message, as a WebSocket's `binaryType`. */
type SocketBinaryType = 'blob' | 'arraybuffer'

/** The part of a WebSocket the connection uses. */
export interface ConnectionSocket {
  binaryType: SocketBinaryType
  readonly bufferedAmount: number
  send(data: Uint8Array<ArrayBuffer>): void
  close(): void
  addEventListener(type: 'open', listener: () => void): void
  addEventListener(type: 'message', listener: (event: { data: unknown }) => void): void
  addEventListener(type: 'close', listener: (event: { code: number; reason: string }) => void): void
}

/** A watch over the open socket's silence. */
export interface SilenceWatch {
  /** The connection brought a message: its silence starts again. */
  heard(): void
  /** The connection brought the core's `pong` to the ping `id`. */
  ponged(id: number): void
  /** Ends the watch; nothing is called after it. */
  stop(): void
}

/** What the connection logs, under an event's name, of a lane's conversation when it has one. */
export interface ConnectionLog {
  info(event: string, fields: Record<string, unknown>, conversation?: string): void
  warn(event: string, fields: Record<string, unknown>, conversation?: string): void
}

/** Where a connection runs: its socket, its liveness and its log. */
export interface ConnectionHost {
  /** Opens the connection's socket. */
  open(): ConnectionSocket
  /**
   * Watches the open socket: `broken` once it has brought nothing for too
   * long, and `ping` to ask the core for a `pong` of the same id.
   */
  watch(broken: () => void, ping: (id: number) => void): SilenceWatch
  /**
   * Calls `connect` once, after the wait for `failures` connections in a row
   * that ended or could not be made, or, `restarting`, the wait for a core
   * that said it restarts.
   */
  wait(failures: number, restarting: boolean, connect: () => void): { cancel(): void }
  /** The connection reaches the core again after it was lost. */
  reached(): void
  /** The connection could not be made, or was lost before it counted as made. */
  failedToOpen(): void
  /** Where the connection stands changed. */
  changed(standing: ConnectionStanding): void
  log: ConnectionLog
}

/** Where a connection stands. */
export interface ConnectionStanding {
  /** Connections in a row that were lost or could not be made before one counted as connected. */
  failures: number
  /** The connection closed with 1001 `backend_closing`, and none counted as connected since. */
  restarting: boolean
}

/** A lane's message the client has yet to send. */
interface Outgoing {
  bytes: Uint8Array<ArrayBuffer>
  binary: boolean
}

/** Whose pieces the client writes first: a user stream's go after every other lane's. */
type Priority = 0 | 1

/** A lane's close, as a WebSocket's: its code, its reason, and whether it was clean. */
export class LaneCloseEvent extends Event {
  constructor(readonly code: number, readonly reason: string) {
    super('close')
  }

  get wasClean(): boolean {
    return this.code !== ABNORMAL_CLOSURE
  }
}

/** The events a lane dispatches, as a WebSocket's. */
export interface LaneEventMap {
  open: Event
  message: MessageEvent
  error: Event
  close: LaneCloseEvent
}

// The events' types for the listeners of a lane, as `WebSocket` declares its own.
export interface Lane {
  addEventListener<K extends keyof LaneEventMap>(type: K, listener: (event: LaneEventMap[K]) => void, options?: boolean | AddEventListenerOptions): void
  removeEventListener<K extends keyof LaneEventMap>(type: K, listener: (event: LaneEventMap[K]) => void, options?: boolean | EventListenerOptions): void
}

/** A lane of a page connection, with the part of WebSocket's interface its owners use. */
export class Lane extends EventTarget {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readonly CONNECTING = 0
  readonly OPEN = 1
  readonly CLOSING = 2
  readonly CLOSED = 3

  readyState: number = Lane.CONNECTING
  binaryType: SocketBinaryType = 'blob'
  /** Why the core refused the open, when it did. */
  refusal: LaneRefusal | null = null
  /** Its number on the connection its open went on; 0 while the open waits. */
  id = 0
  /** How many more of its messages the core takes. */
  credit = LANE_CREDIT
  /** Its messages not started yet. */
  outgoing: Outgoing[] = []
  /**
   * The messages sent before it opened, which the core holds until it
   * answers the open: a connection lost meanwhile took them, and the next
   * sends them again behind the lane's open.
   */
  readonly early: Outgoing[] = []
  /** The message whose pieces it sends, and how much of it went. */
  sending: (Outgoing & { at: number }) | null = null
  /** The pieces of the core's message that more pieces complete. */
  assembling: { pieces: Uint8Array<ArrayBuffer>[]; binary: boolean } | null = null
  openTimer: ReturnType<typeof setTimeout> | null = null
  /** When its open went, and when it opened, by `performance.now()`. */
  openedAt = 0
  sentAt = 0
  readonly priority: Priority

  constructor(readonly connection: PageConnection, readonly target: LaneTarget, readonly name: string) {
    super()
    this.priority = target.kind === 'user_stream' ? 1 : 0
  }

  /** The conversation the lane is of, which its log lines name. */
  get conversation(): string | undefined {
    return 'conversation' in this.target ? this.target.conversation : undefined
  }

  /** The bytes its messages hold that are still to send. */
  get bufferedAmount(): number {
    const sending = this.sending === null ? 0 : this.sending.bytes.length - this.sending.at
    return this.outgoing.reduce((total, message) => total + message.bytes.length, sending)
  }

  /**
   * Sends a message, which may follow the lane's open before `opened`
   * comes, as a stream's data follows its headers in HTTP/2: the core
   * holds it until it answers the open, and drops it with a refusal.
   */
  send(data: string | ArrayBuffer | ArrayBufferView): void {
    // A closed lane takes nothing more, as a closed WebSocket does.
    if (this.readyState !== Lane.OPEN && this.readyState !== Lane.CONNECTING) {
      return
    }
    // A copy of the caller's bytes, which it may change once the call
    // returns, as a WebSocket's send takes them.
    const message: Outgoing = typeof data === 'string'
      ? { bytes: new TextEncoder().encode(data), binary: false }
      : { bytes: data instanceof ArrayBuffer ? new Uint8Array(data.slice(0)) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength).slice(), binary: true }
    this.outgoing.push(message)
    if (this.readyState === Lane.CONNECTING) {
      this.early.push(message)
    }
    this.connection.ready(this)
    this.connection.flush()
  }

  close(code = 1000, reason = ''): void {
    if (this.readyState === Lane.CLOSING || this.readyState === Lane.CLOSED) {
      return
    }
    const opening = this.readyState === Lane.CONNECTING
    this.readyState = Lane.CLOSING
    this.connection.forget(this, code, reason)
    // As a WebSocket's, the close reaches the owner after the call: one
    // that was opening as a failure.
    setTimeout(() => this.connection.finish(this, opening ? ABNORMAL_CLOSURE : code, opening ? '' : reason, 'page'), 0)
  }
}

/** A page connection to one core, and its lanes. */
export class PageConnection {
  /** Every lane not closed yet. */
  private readonly lanes = new Set<Lane>()
  /** The lanes whose open went on the current connection, by number. */
  private readonly numbered = new Map<number, Lane>()
  /** The number of the last lane opened on the current connection. */
  private lastId = 0
  /** The lanes with a piece to send, in turn, by priority. */
  private readonly turns: [Lane[], Lane[]] = [[], []]
  /** The connection's own messages and the lanes' opens and closes, in order. */
  private control: Uint8Array<ArrayBuffer>[] = []
  private socket: ConnectionSocket | null = null
  /** Whether `socket` opened. */
  private socketOpen = false
  /** Whether the connection may connect: its host has what it needs. */
  private started = false
  /** The wait before the connection connects again, while it is closed. */
  private retry: { cancel(): void } | null = null
  private silence: SilenceWatch | null = null
  /** Ends the open connection's grace, which `established` ends first when it comes. */
  private grace: ReturnType<typeof setTimeout> | null = null
  private drain: ReturnType<typeof setTimeout> | null = null
  private readonly standing: ConnectionStanding = { failures: 0, restarting: false }

  constructor(private readonly host: ConnectionHost) {}

  /** Lets the connection connect; a lane asked for before waits for it. */
  start(): void {
    this.started = true
    if (this.lanes.size > 0) {
      this.connect()
    }
  }

  /**
   * Ends the connection for good: every lane closes as the connection's loss
   * closes it, and none connects again.
   */
  end(): void {
    this.started = false
    this.retry?.cancel()
    this.retry = null
    this.silence?.stop()
    this.silence = null
    if (this.grace !== null) {
      clearTimeout(this.grace)
      this.grace = null
    }
    // The socket's own close comes to no connection.
    const socket = this.socket
    this.socket = null
    this.socketOpen = false
    this.numbered.clear()
    this.control = []
    this.turns[0].length = 0
    this.turns[1].length = 0
    for (const lane of [...this.lanes]) {
      this.finish(lane, ABNORMAL_CLOSURE, '', 'connection')
    }
    socket?.close()
  }

  /** Opens a lane that carries what `target` names, logged as `name`, such as "conversation lane". */
  openLane(target: LaneTarget, name: string): Lane {
    const lane = new Lane(this, target, name)
    this.lanes.add(lane)
    if (this.socket !== null && this.socketOpen) {
      this.sendOpen(lane)
    } else {
      this.connect()
    }
    return lane
  }

  /** Connects, unless the connection is open or opening, or waits to connect. */
  private connect(): void {
    if (this.socket !== null || !this.started || this.retry !== null) {
      return
    }
    const opening = this.host.open()
    opening.binaryType = 'arraybuffer'
    this.socket = opening
    this.socketOpen = false
    this.lastId = 0
    opening.addEventListener('open', () => {
      if (this.socket !== opening) {
        return
      }
      this.socketOpen = true
      this.silence = this.host.watch(() => this.lose(opening), (id) => {
        this.control.push(connectionFrame({ type: 'ping', id }))
        this.flush()
      })
      this.grace = setTimeout(() => this.established(), ESTABLISHED_MS)
      for (const lane of this.lanes) {
        if (lane.readyState === Lane.CONNECTING) {
          this.sendOpen(lane)
        }
      }
    })
    opening.addEventListener('message', (event) => {
      if (this.socket !== opening) {
        return
      }
      this.silence?.heard()
      this.established()
      if (!(event.data instanceof ArrayBuffer) || !this.receive(new Uint8Array(event.data))) {
        this.host.log.warn('connection.protocol_error', {})
        this.lose(opening)
      }
    })
    opening.addEventListener('close', (event) => {
      if (this.socket === opening) {
        this.lost(event.code, event.reason)
      }
    })
  }

  /** Lets the connection go, as broken, and connects again. */
  private lose(which: ConnectionSocket): void {
    if (this.socket !== which) {
      return
    }
    this.lost(ABNORMAL_CLOSURE, '')
    which.close()
  }

  /**
   * The connection ended with `code` and `reason`: each open lane closes with
   * them, each lane still opening waits for the next connection, and the
   * connection connects again while a lane wants it.
   */
  private lost(code: number, reason: string): void {
    const opened = this.socketOpen
    this.socket = null
    this.socketOpen = false
    this.silence?.stop()
    this.silence = null
    if (this.grace !== null) {
      clearTimeout(this.grace)
      this.grace = null
    }
    this.numbered.clear()
    this.control = []
    this.turns[0].length = 0
    this.turns[1].length = 0
    this.standing.failures += 1
    if (code === BACKEND_CLOSING) {
      this.standing.restarting = true
    }
    this.host.changed({ ...this.standing })
    for (const lane of [...this.lanes]) {
      if (lane.readyState === Lane.CONNECTING) {
        if (lane.openTimer !== null) {
          clearTimeout(lane.openTimer)
          lane.openTimer = null
        }
        lane.id = 0
        // What it sent went with the connection: the next one carries it again.
        lane.outgoing = [...lane.early]
        lane.sending = null
        lane.credit = LANE_CREDIT
      } else {
        this.finish(lane, code, reason, 'connection')
      }
    }
    if (!opened) {
      this.host.failedToOpen()
    }
    if (this.lanes.size > 0 && this.started) {
      this.retry = this.host.wait(this.standing.failures, this.standing.restarting, () => {
        this.retry = null
        if (this.lanes.size > 0) {
          this.connect()
        }
      })
    }
  }

  /**
   * The open connection stayed open, or brought a message: the core is
   * reached, and the host hears it after a loss.
   */
  private established(): void {
    if (this.grace !== null) {
      clearTimeout(this.grace)
      this.grace = null
    }
    const returned = this.standing.failures > 0 || this.standing.restarting
    if (!returned) {
      return
    }
    this.standing.failures = 0
    this.standing.restarting = false
    this.host.changed({ ...this.standing })
    this.host.reached()
  }

  private sendOpen(lane: Lane): void {
    this.lastId += 1
    lane.id = this.lastId
    this.numbered.set(lane.id, lane)
    lane.sentAt = performance.now()
    this.control.push(frame(OPEN_FRAME, lane.id, new TextEncoder().encode(JSON.stringify(lane.target))))
    lane.openTimer = setTimeout(() => {
      lane.openTimer = null
      if (lane.readyState !== Lane.CONNECTING) {
        return
      }
      this.host.log.warn('lane.failed', { lane: lane.name, why: 'no answer to its open within 10 seconds' }, lane.conversation)
      // The core forgets it, should its answer still come.
      this.control.push(closeFrame(lane.id, ABNORMAL_CLOSURE, ''))
      this.flush()
      this.finish(lane, ABNORMAL_CLOSURE, '', 'page')
    }, OPEN_MS)
    this.ready(lane)
    this.flush()
  }

  /**
   * The core forgets `lane`, which its owner closes, an open it still
   * checks included; one whose open has not gone needs no word.
   */
  forget(lane: Lane, code: number, reason: string): void {
    if (lane.id !== 0 && this.socket !== null) {
      this.control.push(closeFrame(lane.id, code, reason))
      this.flush()
    }
  }

  /**
   * Ends `lane` with `code` and `reason`, which `by` ended, and tells its
   * owner: a lane that never opened as a failure.
   */
  finish(lane: Lane, code: number, reason: string, by: 'page' | 'backend' | 'connection'): void {
    if (lane.readyState === Lane.CLOSED) {
      return
    }
    const opened = lane.openedAt !== 0
    lane.readyState = Lane.CLOSED
    if (lane.openTimer !== null) {
      clearTimeout(lane.openTimer)
      lane.openTimer = null
    }
    this.lanes.delete(lane)
    if (this.numbered.get(lane.id) === lane) {
      this.numbered.delete(lane.id)
    }
    lane.outgoing.length = 0
    lane.early.length = 0
    lane.sending = null
    lane.assembling = null
    for (const turn of this.turns) {
      const at = turn.indexOf(lane)
      if (at !== -1) {
        turn.splice(at, 1)
      }
    }
    if (opened) {
      this.host.log.info('lane.closed', { lane: lane.name, code, reason, by, life_ms: Math.round(performance.now() - lane.openedAt) }, lane.conversation)
    }
    if (code === ABNORMAL_CLOSURE) {
      lane.dispatchEvent(new Event('error'))
    }
    lane.dispatchEvent(new LaneCloseEvent(code, reason))
  }

  /** Takes one frame of the core's; false when it breaks the protocol. */
  private receive(bytes: Uint8Array): boolean {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    const kind = bytes[0]
    if (kind === CONNECTION_FRAME) {
      const message = backendConnectionMessageSchema.safeParse(json(bytes.subarray(1)))
      if (!message.success) {
        return false
      }
      if (message.data.type === 'pong') {
        this.silence?.ponged(message.data.id)
      }
      return true
    }
    if (bytes.length < 5) {
      return false
    }
    // A frame of a lane that ended meanwhile goes nowhere.
    const lane = this.numbered.get(view.getUint32(1))
    switch (kind) {
      case OPENED_FRAME:
        if (lane?.readyState === Lane.CONNECTING) {
          if (lane.openTimer !== null) {
            clearTimeout(lane.openTimer)
            lane.openTimer = null
          }
          lane.readyState = Lane.OPEN
          lane.early.length = 0
          lane.openedAt = performance.now()
          this.host.log.info('lane.opened', { lane: lane.name, open_ms: Math.round(lane.openedAt - lane.sentAt) }, lane.conversation)
          lane.dispatchEvent(new Event('open'))
        }
        return true
      case REFUSED_FRAME: {
        const refusal = laneRefusalSchema.safeParse(json(bytes.subarray(5)))
        if (!refusal.success) {
          return false
        }
        if (lane?.readyState === Lane.CONNECTING) {
          lane.refusal = refusal.data
          const { status, error } = refusal.data
          this.host.log.info('lane.refused', { lane: lane.name, status, code: error.code, request_id: error.requestId }, lane.conversation)
          this.finish(lane, ABNORMAL_CLOSURE, '', 'backend')
        }
        return true
      }
      case DATA_FRAME: {
        if (bytes.length < 6) {
          return false
        }
        if (lane?.readyState === Lane.OPEN) {
          take(lane, bytes[5]!, bytes.slice(6))
        }
        return true
      }
      case CLOSE_FRAME: {
        if (bytes.length < 7) {
          return false
        }
        if (lane !== undefined) {
          this.finish(lane, view.getUint16(5), new TextDecoder().decode(bytes.subarray(7)), 'backend')
        }
        return true
      }
      case CREDIT_FRAME: {
        if (bytes.length < 7) {
          return false
        }
        if (lane !== undefined) {
          lane.credit += view.getUint16(5)
          this.ready(lane)
          this.flush()
        }
        return true
      }
      default:
        return false
    }
  }

  /** Puts `lane` in its turn when it has a piece to send and was not there. */
  ready(lane: Lane): void {
    const waits = lane.sending !== null || (lane.outgoing.length > 0 && lane.credit > 0)
    const turn = this.turns[lane.priority]
    // A lane still opening sends behind its open, once that went on this connection.
    const sends = lane.readyState === Lane.OPEN || (lane.readyState === Lane.CONNECTING && lane.id !== 0)
    if (waits && sends && !turn.includes(lane)) {
      turn.push(lane)
    }
  }

  /** The next lane's next piece, in turn; null when none has one. */
  private nextPiece(): Uint8Array<ArrayBuffer> | null {
    for (const turn of this.turns) {
      const lane = turn.shift()
      if (lane === undefined) {
        continue
      }
      if (lane.sending === null) {
        const next = lane.outgoing.shift()!
        lane.credit -= 1
        lane.sending = { ...next, at: 0 }
      }
      const { bytes, binary, at } = lane.sending
      const end = Math.min(at + LANE_PIECE_BYTES, bytes.length)
      const more = end < bytes.length
      const header = new Uint8Array(6 + end - at)
      new DataView(header.buffer).setUint32(1, lane.id)
      header[0] = DATA_FRAME
      header[5] = (binary ? BINARY_FLAG : 0) | (more ? MORE_FLAG : 0)
      header.set(bytes.subarray(at, end), 6)
      if (more) {
        lane.sending.at = end
      } else {
        lane.sending = null
      }
      this.ready(lane)
      return header
    }
    return null
  }

  /** Writes what waits: every control message, then pieces while the socket has room. */
  flush(): void {
    const socket = this.socket
    if (socket === null || !this.socketOpen) {
      return
    }
    for (const message of this.control) {
      socket.send(message)
    }
    this.control = []
    while (socket.bufferedAmount < BUFFERED_BYTES) {
      const piece = this.nextPiece()
      if (piece === null) {
        return
      }
      socket.send(piece)
    }
    // The socket holds enough: the rest waits for it to send what it holds,
    // which a WebSocket does not announce.
    this.drain ??= setTimeout(() => {
      this.drain = null
      this.flush()
    }, DRAIN_MS)
  }
}

/** Takes a piece of the core's message on `lane`, and hands the owner the message it completes. */
function take(lane: Lane, flags: number, piece: Uint8Array<ArrayBuffer>): void {
  const binary = (flags & BINARY_FLAG) !== 0
  if ((flags & MORE_FLAG) !== 0) {
    lane.assembling ??= { pieces: [], binary }
    lane.assembling.pieces.push(piece)
    return
  }
  let whole = piece
  if (lane.assembling !== null) {
    const pieces = [...lane.assembling.pieces, piece]
    lane.assembling = null
    whole = new Uint8Array(pieces.reduce((total, each) => total + each.length, 0))
    let at = 0
    for (const each of pieces) {
      whole.set(each, at)
      at += each.length
    }
  }
  const data = !binary
    ? new TextDecoder().decode(whole)
    : lane.binaryType === 'arraybuffer'
      ? whole.buffer
      : new Blob([whole])
  lane.dispatchEvent(new MessageEvent('message', { data }))
}

function frame(kind: number, lane: number, body: Uint8Array): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(5 + body.length)
  bytes[0] = kind
  new DataView(bytes.buffer).setUint32(1, lane)
  bytes.set(body, 5)
  return bytes
}

function closeFrame(lane: number, code: number, reason: string): Uint8Array<ArrayBuffer> {
  const text = new TextEncoder().encode(reason)
  const body = new Uint8Array(2 + text.length)
  new DataView(body.buffer).setUint16(0, code)
  body.set(text, 2)
  return frame(CLOSE_FRAME, lane, body)
}

function connectionFrame(message: PageConnectionMessage): Uint8Array<ArrayBuffer> {
  const body = new TextEncoder().encode(JSON.stringify(message))
  const bytes = new Uint8Array(1 + body.length)
  bytes[0] = CONNECTION_FRAME
  bytes.set(body, 1)
  return bytes
}

function json(bytes: Uint8Array): unknown {
  try {
    return JSON.parse(new TextDecoder().decode(bytes))
  } catch {
    return undefined
  }
}

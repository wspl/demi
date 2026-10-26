/**
 * The page connection: the one WebSocket the page holds to the backend, and
 * the lanes it carries (`crates/web-api-protocol/src/connection.rs`).
 *
 * Each part of the page that talks with the backend over a socket opens a
 * lane: the synchronization channel, each conversation's lane, the file
 * watches, a device's signaling and the user streams. A lane looks to its
 * owner as a WebSocket does: it opens or fails, carries messages, and closes
 * with a code and a reason; a lane the backend refused says why.
 *
 * ```text
 *  owners --- openLane ---> lanes --- one queue, in turn ---> WebSocket
 *     ^                                                          |
 *     `---------- open, message, close events <---- frames ------'
 * ```
 *
 * - The connection opens as the first lane is asked for, and connects again
 *   after it is lost, while a lane wants it, after waits that double, or at
 *   the page's return (`liveness.ts`). As it opens again, every owner that
 *   waits to connect connects at once.
 * - Its liveness is the page's: the backend's heartbeat after 30 silent
 *   seconds, the 75-second silence, and the ping at the page's return. A lane
 *   has none of its own.
 * - A lane asked for while the connection is down waits for it, and its
 *   open's 10 seconds start as the open goes. A lost connection closes each
 *   open lane with the connection's close, 1006 when it broke, and sends each
 *   lane still opening again on the next connection.
 * - The page writes the connection's own messages and the lanes' opens and
 *   closes first, then one piece of each lane's message in turn, a user
 *   stream's after the others', and a piece only while the socket holds less
 *   than 64 KiB unsent. A lane starts no more of its messages than the
 *   backend's credit allows.
 */
import { readonly, ref } from 'vue'
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
import { pageLog } from '../infra/log'
import { openSocket, reconnectNow, waitForConnection, watchSilence, type ReconnectWait, type SilenceWatch } from './liveness'

export type { LaneRefusal, LaneTarget }

/** How long a lane's open may wait for its answer once it went. */
const OPEN_MS = 10_000
/** How much the socket may hold unsent before a lane's next piece waits. */
const BUFFERED_BYTES = 64 * 1024
/** How long a piece waits for the socket to send what it holds before the page looks again. */
const DRAIN_MS = 16
/**
 * How long an open connection that brought nothing must stay open before it
 * counts as connected: a backend that accepts the upgrade and closes it at
 * once is not retried at the rate of a quiet network.
 */
const ESTABLISHED_MS = 1_000
/** The close of a connection whose backend shuts down, and will be back. */
const BACKEND_CLOSING = 1001
/** The close a lane hears of a connection that ended without a close, or of an open that failed. */
const ABNORMAL_CLOSURE = 1006

/** What the page tells the connection once, as it starts. */
export interface PageConnectionOptions {
  /** The connection's address, `/api/connect`, as the page reaches the backend. */
  url: () => string
  /**
   * The connection could not be made. The web browser does not say why: an
   * ended session answers its upgrade with 401, which a request learns.
   */
  failedToOpen: () => void
}

let options: PageConnectionOptions | null = null

/** Starts the connection with `given`; a lane asked for before waits for it. */
export function configurePageConnection(given: PageConnectionOptions): void {
  options = given
  if (lanes.size > 0) {
    connect()
  }
}

/** Connections in a row that were lost or could not be made before one counted as connected. */
const failures = ref(0)
/** The connection closed with 1001 `backend_closing`, and none counted as connected since. */
const restarting = ref(false)
/** Connections in a row that were lost or could not be made, for where the page's connection stands. */
export const connectionFailures = readonly(failures)
/** Whether the backend said it restarts, for where the page's connection stands. */
export const backendRestarting = readonly(restarting)

/** A lane's message the page has yet to send. */
interface Outgoing {
  bytes: Uint8Array<ArrayBuffer>
  binary: boolean
}

/** Whose pieces the page writes first: a user stream's go after every other lane's. */
type Priority = 0 | 1

/** The events a lane dispatches, as a WebSocket's. */
export interface LaneEventMap {
  open: Event
  message: MessageEvent
  error: Event
  close: CloseEvent
}

// The events' types for the listeners of a lane, as `WebSocket` declares its own.
export interface Lane {
  addEventListener<K extends keyof LaneEventMap>(type: K, listener: (event: LaneEventMap[K]) => void, options?: boolean | AddEventListenerOptions): void
  removeEventListener<K extends keyof LaneEventMap>(type: K, listener: (event: LaneEventMap[K]) => void, options?: boolean | EventListenerOptions): void
}

/** A lane of the page connection, with the part of WebSocket's interface its owners use. */
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
  binaryType: BinaryType = 'blob'
  /** Why the backend refused the open, when it did. */
  refusal: LaneRefusal | null = null
  /** Its number on the connection its open went on; 0 while the open waits. */
  id = 0
  /** How many more of its messages the backend takes. */
  credit = LANE_CREDIT
  /** Its messages not started yet. */
  readonly outgoing: Outgoing[] = []
  /** The message whose pieces it sends, and how much of it went. */
  sending: (Outgoing & { at: number }) | null = null
  /** The pieces of the backend's message that more pieces complete. */
  assembling: { pieces: Uint8Array<ArrayBuffer>[]; binary: boolean } | null = null
  openTimer: ReturnType<typeof setTimeout> | null = null
  /** When its open went, and when it opened, by `performance.now()`. */
  openedAt = 0
  sentAt = 0
  readonly priority: Priority

  constructor(readonly target: LaneTarget, readonly name: string) {
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

  send(data: string | ArrayBuffer | ArrayBufferView): void {
    if (this.readyState === Lane.CONNECTING) {
      throw new DOMException('The lane is still opening', 'InvalidStateError')
    }
    // A closed lane takes nothing more, as a closed WebSocket does.
    if (this.readyState !== Lane.OPEN) {
      return
    }
    // A copy of the caller's bytes, which it may change once the call
    // returns, as a WebSocket's send takes them.
    const message: Outgoing = typeof data === 'string'
      ? { bytes: new TextEncoder().encode(data), binary: false }
      : { bytes: data instanceof ArrayBuffer ? new Uint8Array(data.slice(0)) : new Uint8Array(data.buffer, data.byteOffset, data.byteLength).slice(), binary: true }
    this.outgoing.push(message)
    ready(this)
    flush()
  }

  close(code = 1000, reason = ''): void {
    if (this.readyState === Lane.CLOSING || this.readyState === Lane.CLOSED) {
      return
    }
    const opening = this.readyState === Lane.CONNECTING
    this.readyState = Lane.CLOSING
    // The backend forgets the lane, an open it still checks included; one
    // whose open has not gone needs no word.
    if (this.id !== 0 && socket !== null) {
      control.push(closeFrame(this.id, code, reason))
      flush()
    }
    // As a WebSocket's, the close reaches the owner after the call: one
    // that was opening as a failure.
    setTimeout(() => finish(this, opening ? ABNORMAL_CLOSURE : code, opening ? '' : reason, 'page'), 0)
  }
}

/** Opens a lane that carries what `target` names, logged as `name`, such as "conversation lane". */
export function openLane(target: LaneTarget, name: string): Lane {
  const lane = new Lane(target, name)
  lanes.add(lane)
  if (socket !== null && socketOpen) {
    sendOpen(lane)
  } else {
    connect()
  }
  return lane
}

/** Every lane not closed yet. */
const lanes = new Set<Lane>()
/** The lanes whose open went on the current connection, by number. */
const numbered = new Map<number, Lane>()
/** The number of the last lane opened on the current connection. */
let lastId = 0
/** The lanes with a piece to send, in turn, by priority. */
const turns: [Lane[], Lane[]] = [[], []]
/** The connection's own messages and the lanes' opens and closes, in order. */
let control: Uint8Array<ArrayBuffer>[] = []

let socket: WebSocket | null = null
/** Whether `socket` opened. */
let socketOpen = false
/** The wait before the connection connects again, while it is closed. */
let retry: ReconnectWait | null = null
let silence: SilenceWatch | null = null
/** Ends the open connection's grace, which `established` ends first when it comes. */
let grace: ReturnType<typeof setTimeout> | null = null
let drain: ReturnType<typeof setTimeout> | null = null

/** Connects, unless the connection is open or opening, or waits to connect. */
function connect(): void {
  if (socket !== null || options === null || retry !== null) {
    return
  }
  const url = new URL(options.url(), globalThis.location?.href)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  const opening = openSocket(url, 'page connection')
  opening.binaryType = 'arraybuffer'
  socket = opening
  socketOpen = false
  lastId = 0
  opening.addEventListener('open', () => {
    if (socket !== opening) {
      return
    }
    socketOpen = true
    silence = watchSilence('page connection', () => lose(opening), (id) => {
      control.push(connectionFrame({ type: 'ping', id }))
      flush()
    })
    grace = setTimeout(established, ESTABLISHED_MS)
    for (const lane of lanes) {
      if (lane.readyState === Lane.CONNECTING) {
        sendOpen(lane)
      }
    }
  })
  opening.addEventListener('message', (event: MessageEvent) => {
    if (socket !== opening) {
      return
    }
    silence?.heard()
    established()
    if (!(event.data instanceof ArrayBuffer) || !receive(new Uint8Array(event.data))) {
      pageLog.warn('connection.protocol_error', {})
      lose(opening)
    }
  })
  opening.addEventListener('close', (event: CloseEvent) => {
    if (socket === opening) {
      lost(event.code, event.reason)
    }
  })
}

/** Lets the connection go, as broken, and connects again. */
function lose(which: WebSocket): void {
  if (socket !== which) {
    return
  }
  lost(ABNORMAL_CLOSURE, '')
  which.close()
}

/**
 * The connection ended with `code` and `reason`: each open lane closes with
 * them, each lane still opening waits for the next connection, and the
 * connection connects again while a lane wants it.
 */
function lost(code: number, reason: string): void {
  const opened = socketOpen
  socket = null
  socketOpen = false
  silence?.stop()
  silence = null
  if (grace !== null) {
    clearTimeout(grace)
    grace = null
  }
  numbered.clear()
  control = []
  turns[0].length = 0
  turns[1].length = 0
  failures.value += 1
  if (code === BACKEND_CLOSING) {
    restarting.value = true
  }
  for (const lane of [...lanes]) {
    if (lane.readyState === Lane.CONNECTING) {
      if (lane.openTimer !== null) {
        clearTimeout(lane.openTimer)
        lane.openTimer = null
      }
      lane.id = 0
    } else {
      finish(lane, code, reason, 'connection')
    }
  }
  if (!opened) {
    options?.failedToOpen()
  }
  if (lanes.size > 0) {
    retry = waitForConnection(failures.value, restarting.value, () => {
      retry = null
      if (lanes.size > 0) {
        connect()
      }
    })
  }
}

/**
 * The open connection stayed open, or brought a message: the backend is
 * reached, and each owner that waits to connect connects at once.
 */
function established(): void {
  if (grace !== null) {
    clearTimeout(grace)
    grace = null
  }
  const returned = failures.value > 0 || restarting.value
  failures.value = 0
  restarting.value = false
  if (returned) {
    reconnectNow()
  }
}

function sendOpen(lane: Lane): void {
  lastId += 1
  lane.id = lastId
  numbered.set(lane.id, lane)
  lane.sentAt = performance.now()
  control.push(frame(OPEN_FRAME, lane.id, new TextEncoder().encode(JSON.stringify(lane.target))))
  lane.openTimer = setTimeout(() => {
    lane.openTimer = null
    if (lane.readyState !== Lane.CONNECTING) {
      return
    }
    pageLog.warn('lane.failed', { lane: lane.name, why: 'no answer to its open within 10 seconds' }, lane.conversation)
    // The backend forgets it, should its answer still come.
    control.push(closeFrame(lane.id, ABNORMAL_CLOSURE, ''))
    flush()
    finish(lane, ABNORMAL_CLOSURE, '', 'page')
  }, OPEN_MS)
  flush()
}

/**
 * Ends `lane` with `code` and `reason`, which `by` ended, and tells its
 * owner: a lane that never opened as a failure.
 */
function finish(lane: Lane, code: number, reason: string, by: 'page' | 'backend' | 'connection'): void {
  if (lane.readyState === Lane.CLOSED) {
    return
  }
  const opened = lane.openedAt !== 0
  lane.readyState = Lane.CLOSED
  if (lane.openTimer !== null) {
    clearTimeout(lane.openTimer)
    lane.openTimer = null
  }
  lanes.delete(lane)
  if (numbered.get(lane.id) === lane) {
    numbered.delete(lane.id)
  }
  lane.outgoing.length = 0
  lane.sending = null
  lane.assembling = null
  for (const turn of turns) {
    const at = turn.indexOf(lane)
    if (at !== -1) {
      turn.splice(at, 1)
    }
  }
  if (opened) {
    pageLog.info('lane.closed', { lane: lane.name, code, reason, by, life_ms: Math.round(performance.now() - lane.openedAt) }, lane.conversation)
  }
  if (code === ABNORMAL_CLOSURE) {
    lane.dispatchEvent(new Event('error'))
  }
  lane.dispatchEvent(new CloseEvent('close', { code, reason, wasClean: code !== ABNORMAL_CLOSURE }))
}

/** Takes one frame of the backend's; false when it breaks the protocol. */
function receive(bytes: Uint8Array): boolean {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const kind = bytes[0]
  if (kind === CONNECTION_FRAME) {
    const message = backendConnectionMessageSchema.safeParse(json(bytes.subarray(1)))
    if (!message.success) {
      return false
    }
    if (message.data.type === 'pong') {
      silence?.ponged(message.data.id)
    }
    return true
  }
  if (bytes.length < 5) {
    return false
  }
  // A frame of a lane that ended meanwhile goes nowhere.
  const lane = numbered.get(view.getUint32(1))
  switch (kind) {
    case OPENED_FRAME:
      if (lane?.readyState === Lane.CONNECTING) {
        if (lane.openTimer !== null) {
          clearTimeout(lane.openTimer)
          lane.openTimer = null
        }
        lane.readyState = Lane.OPEN
        lane.openedAt = performance.now()
        pageLog.info('lane.opened', { lane: lane.name, open_ms: Math.round(lane.openedAt - lane.sentAt) }, lane.conversation)
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
        pageLog.info('lane.refused', { lane: lane.name, status, code: error.code, request_id: error.requestId }, lane.conversation)
        finish(lane, ABNORMAL_CLOSURE, '', 'backend')
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
        finish(lane, view.getUint16(5), new TextDecoder().decode(bytes.subarray(7)), 'backend')
      }
      return true
    }
    case CREDIT_FRAME: {
      if (bytes.length < 7) {
        return false
      }
      if (lane !== undefined) {
        lane.credit += view.getUint16(5)
        ready(lane)
        flush()
      }
      return true
    }
    default:
      return false
  }
}

/** Takes a piece of the backend's message on `lane`, and hands the owner the message it completes. */
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

/** Puts `lane` in its turn when it has a piece to send and was not there. */
function ready(lane: Lane): void {
  const waits = lane.sending !== null || (lane.outgoing.length > 0 && lane.credit > 0)
  const turn = turns[lane.priority]
  if (waits && lane.readyState === Lane.OPEN && !turn.includes(lane)) {
    turn.push(lane)
  }
}

/** The next lane's next piece, in turn; null when none has one. */
function nextPiece(): Uint8Array<ArrayBuffer> | null {
  for (const turn of turns) {
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
    ready(lane)
    return header
  }
  return null
}

/** Writes what waits: every control message, then pieces while the socket has room. */
function flush(): void {
  if (socket === null || !socketOpen) {
    return
  }
  for (const message of control) {
    socket.send(message)
  }
  control = []
  while (socket.bufferedAmount < BUFFERED_BYTES) {
    const piece = nextPiece()
    if (piece === null) {
      return
    }
    socket.send(piece)
  }
  // The socket holds enough: the rest waits for it to send what it holds,
  // which the browser does not announce.
  drain ??= setTimeout(() => {
    drain = null
    flush()
  }, DRAIN_MS)
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


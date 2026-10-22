/**
 * The page's lines of the backend's log.
 *
 * ```text
 *   pageLog.info('view.opened', {...})  -->  buffer
 *                                              |  every 5 s, at once for an error
 *                                              v
 *                                       sender: POST /log
 *   pagehide / hidden  --> one `keepalive` request, 16 KiB at most
 * ```
 *
 * - The page names its load with an ID, `PAGE_ID`, which every request carries
 *   in `X-Demi-Page` and every socket in its URL, so the backend's lines of a
 *   request this page made are found beside these lines.
 * - A line names things by ID and counts their sizes: scalars only. Text the
 *   user typed and page content never go into one. Nothing is cut: a field is
 *   as long as the page wrote it.
 * - A line that did not get in is counted, not lost silently: the next request
 *   carries one `log.dropped` line with the count, once. The count survives a
 *   reload when the hidden page could not send everything.
 * - A page that is not signed in, or has yet to choose its password (401, 403),
 *   sends nothing and keeps its lines until the app says it is signed in
 *   (`pageLogSignedIn`).
 * - The page sends no line about its own sending, which would send itself for
 *   ever; the backend's request line for each `POST /log` says it.
 */

import { errorFields, type PageErrorReport } from './errors'

export type LogLevel = 'debug' | 'info' | 'warn' | 'error'

/** A scalar: what a line's fields hold. */
export type LogValue = string | number | boolean | null

export interface PageLogLine {
  /** Milliseconds since the Unix epoch, by the page's clock. */
  at: number
  level: LogLevel
  /** What happened, as `noun.verb` in the past tense. */
  event: string
  conversationId?: string
  fields: Record<string, LogValue>
}

export interface PageLogBody {
  page: string
  lines: PageLogLine[]
}

/** What a request of lines came to. */
export type PageLogAnswer =
  /** Stored; `requestId` is the backend's `X-Request-Id` of the request. */
  | { kind: 'stored'; requestId: string | null }
  /** The backend refused these lines, as not well formed; they are lost. */
  | { kind: 'refused'; status: number; requestId: string | null }
  /** The page is not signed in, or has yet to choose its password: 401 or 403. */
  | { kind: 'unauthorized' }
  /** Nothing came of it, as without a network; the lines wait. */
  | { kind: 'unreached' }

/** Sends one request of lines; `keepalive` for the hidden page's, which the browser finishes after the page is gone. */
export type PageLogSender = (body: PageLogBody, keepalive: boolean) => Promise<PageLogAnswer>

const ALPHABET = '0123456789abcdefghjkmnpqrstvwxyz'

/** A new page load's ID: `pg_` and 16 characters, 80 random bits, as a request's ID is made. */
function makePageId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(10))
  let id = 'pg_'
  let bits = 0
  let held = 0
  for (const byte of bytes) {
    held = (held << 8) | byte
    bits += 8
    while (bits >= 5) {
      bits -= 5
      id += ALPHABET[(held >> bits) & 31]
    }
    held &= (1 << bits) - 1
  }
  return id
}

/** This page load's ID. */
export const PAGE_ID = makePageId()

/** The header every request of the page carries `PAGE_ID` in, and the parameter of a socket's URL. */
export const PAGE_HEADER = 'X-Demi-Page'
export const PAGE_PARAMETER = 'page'

/** How long lines wait for company before they are sent. */
const FLUSH_MS = 5_000
/**
 * The most bytes of one request: half of the backend's JSON body limit, 1 MiB,
 * so that a page that wrote much while it could not send still gets in, in
 * several requests.
 */
const BODY_BYTES = 512 * 1024
/**
 * The most bytes of the request a hidden page sends. A browser refuses a
 * `keepalive` body that takes a page's in-flight total over 64 KiB, and the
 * drafts and preferences the page saves as it closes are sent that way too:
 * the log takes a quarter and leaves them the rest.
 */
const HIDDEN_BYTES = 16 * 1024
/** The event of a failure the page found itself (`logPageError`). */
const ERROR_EVENT = 'error.reported'
const EVENT = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$/
const DROPPED_KEY = 'demi.page-log.dropped'
/**
 * The most lines the page holds while it cannot send: signed out, or without a
 * network, or a page, such as the gallery, that sends none. The oldest go, and the
 * next request says how many (`log.dropped`), so that a page that cannot send does
 * not grow without end.
 */
const MAX_BUFFERED = 5_000

let sender: PageLogSender | null = null
/** Whether the app says the page is signed in, which nothing is sent without. */
let signedIn = false
let buffer: PageLogLine[] = []
let timer: ReturnType<typeof setTimeout> | null = null
/** What was in flight or being sent, so that one send runs at a time. */
let chain: Promise<unknown> = Promise.resolve()
/** The lines that did not get in, and why, for the next request's `log.dropped` line. */
let dropped = { count: readDropped(), why: 'the page was left before they were sent' }
/**
 * The errors still waiting for the request that carries their line, each by the
 * line: the one that sends a line answers its ID, whichever call wrote it.
 */
const awaiting = new Map<PageLogLine, (requestId: string | null) => void>()
/** Whether this hidden period's request is sent already. */
let hiddenSent = false

const encoder = new TextEncoder()

function readDropped(): number {
  try {
    const count = Number(localStorage.getItem(DROPPED_KEY))
    localStorage.removeItem(DROPPED_KEY)
    return Number.isInteger(count) && count > 0 ? count : 0
  } catch {
    return 0
  }
}

function keepDropped(): void {
  try {
    if (dropped.count > 0) {
      localStorage.setItem(DROPPED_KEY, String(dropped.count))
    }
  } catch {
    // Storage may refuse; the count is a diagnostic.
  }
}

function drop(count: number, why: string): void {
  if (count > 0) {
    dropped = { count: dropped.count + count, why }
  }
}

function bytesOf(line: PageLogLine): number {
  return encoder.encode(JSON.stringify(line)).length + 1
}

/** Sends what waits at the next tick, when the page may send. */
function schedule(): void {
  if (timer !== null || !sender || !signedIn) {
    return
  }
  timer = setTimeout(() => {
    timer = null
    void flush()
  }, FLUSH_MS)
}

/**
 * `fields` as scalars, whole. A value that is not a scalar, or is undefined, is
 * left out, and a number that is not finite is null, so that one bad field
 * costs neither the line nor the request.
 */
function scalars(fields: Record<string, unknown>): Record<string, LogValue> {
  const kept: Record<string, LogValue> = {}
  for (const [name, value] of Object.entries(fields)) {
    if (typeof value === 'string' || typeof value === 'boolean' || value === null) {
      kept[name] = value
    } else if (typeof value === 'number') {
      kept[name] = Number.isFinite(value) ? value : null
    }
  }
  return kept
}

/**
 * Writes a line, which waits for the next send; an `error` goes at once.
 * Answers the backend's request ID of the request that carried the line, or
 * null when the backend refused it. A page that cannot send yet answers when
 * the line is sent.
 */
function write(
  level: LogLevel,
  event: string,
  fields: Record<string, unknown>,
  conversationId?: string,
): Promise<string | null> {
  const valid = EVENT.test(event)
  const line: PageLogLine = {
    at: Date.now(),
    level,
    event: valid ? event : 'log.invalid_event',
    fields: scalars(valid ? fields : { event }),
  }
  if (conversationId !== undefined && conversationId.length > 0) {
    line.conversationId = conversationId
  }
  buffer.push(line)
  if (buffer.length > MAX_BUFFERED) {
    const oldest = buffer.shift()!
    settle([oldest], null)
    drop(1, 'the page held more lines than it keeps while it could not send')
  }
  if (level === 'error') {
    const carried = new Promise<string | null>((resolve) => awaiting.set(line, resolve))
    void flush()
    return carried
  }
  schedule()
  return Promise.resolve(null)
}

/** What a request of lines is made of. */
interface Taken {
  /** The request's lines: the `log.dropped` line first, when lines were lost. */
  body: PageLogLine[]
  /** The lines taken from the buffer, which a request that came to nothing gives back. */
  real: PageLogLine[]
  /** The count of lost lines the `log.dropped` line carries, to give back or lose again. */
  carried: number
  why: string
}

/** The lines `buffer` starts with that fit `limit` bytes, after the dropped count's line. */
function take(limit: number, atLeastOne: boolean): Taken {
  const body: PageLogLine[] = []
  const real: PageLogLine[] = []
  const carried = dropped.count
  const why = dropped.why
  let size = encoder.encode(JSON.stringify({ page: PAGE_ID, lines: [] })).length
  if (carried > 0) {
    const line: PageLogLine = {
      at: Date.now(),
      level: 'warn',
      event: 'log.dropped',
      fields: { dropped: carried, why },
    }
    body.push(line)
    size += bytesOf(line)
    dropped = { count: 0, why }
  }
  while (buffer.length > 0) {
    const next = bytesOf(buffer[0]!)
    if (size + next > limit && (real.length > 0 || !atLeastOne)) {
      break
    }
    size += next
    const line = buffer.shift()!
    body.push(line)
    real.push(line)
  }
  return { body, real, carried, why }
}

/** Gives back what a request that came to nothing took, ahead of what came since. */
function giveBack(taken: Taken): void {
  drop(taken.carried, taken.why)
  buffer = [...taken.real, ...buffer]
}

/** Answers each error among `lines` with the ID of the request that carried it, or null. */
function settle(lines: PageLogLine[], requestId: string | null): void {
  for (const line of lines) {
    awaiting.get(line)?.(requestId)
    awaiting.delete(line)
  }
}

/** What a refused request lost, counted once: its lines, and the lost lines it carried the count of. */
function lose(taken: Taken, why: string): void {
  drop(taken.real.length + taken.carried, why)
}

/** Sends what the buffer holds, a request of at most `BODY_BYTES` at a time. */
function flush(): Promise<void> {
  const run = chain.then(async () => {
    const send = sender
    while (send && signedIn && buffer.length + dropped.count > 0) {
      const taken = take(BODY_BYTES, true)
      // A request that failed outright is one that reached nothing.
      const answer = await send({ page: PAGE_ID, lines: taken.body }, false).catch((): PageLogAnswer => ({ kind: 'unreached' }))
      if (answer.kind === 'unreached') {
        giveBack(taken)
        schedule()
        return
      }
      if (answer.kind === 'unauthorized') {
        // Nothing is sent until the app says the page is signed in.
        giveBack(taken)
        signedIn = false
        return
      }
      if (answer.kind === 'refused') {
        lose(taken, `the backend refused them with ${answer.status}`)
        settle(taken.real, null)
      } else {
        settle(taken.real, answer.requestId)
      }
    }
    if (signedIn && buffer.length + dropped.count > 0) {
      schedule()
    }
  })
  const settled = run.catch(() => undefined)
  chain = settled
  return settled
}

/** The hidden page's one request: what fits 16 KiB, `keepalive`; the rest is counted for the next load. */
function sendHidden(): void {
  if (hiddenSent || !sender) {
    return
  }
  hiddenSent = true
  if (buffer.length + dropped.count === 0) {
    return
  }
  if (!signedIn) {
    // The lines would be refused: they are counted for the next load.
    drop(buffer.length, 'the page was hidden while it could not send')
    settle(buffer, null)
    buffer = []
    keepDropped()
    return
  }
  const taken = take(HIDDEN_BYTES, false)
  drop(buffer.length, 'the page was hidden with more than a request holds')
  settle(buffer, null)
  buffer = []
  keepDropped()
  if (taken.body.length === 0) {
    return
  }
  // The answer is nobody's: the page may be gone.
  void sender({ page: PAGE_ID, lines: taken.body }, true).then(
    (answer) => {
      if (answer.kind !== 'stored') {
        lose(taken, "the hidden page's request did not get in")
      }
      settle(taken.real, answer.kind === 'stored' ? answer.requestId : null)
      keepDropped()
    },
    () => {
      lose(taken, "the hidden page's request did not get in")
      settle(taken.real, null)
      keepDropped()
    },
  )
}

/**
 * Sends the page's lines with `send`, which the app supplies, once the app
 * says the page is signed in (`pageLogSignedIn`): every 5 seconds while there
 * are lines, at once for an error, and when the page is hidden. Returns the
 * undo.
 */
export function logPageWith(send: PageLogSender): () => void {
  sender = send
  const hide = () => sendHidden()
  const visibility = () => {
    if (globalThis.document?.visibilityState === 'hidden') {
      sendHidden()
    } else {
      hiddenSent = false
      // The page goes on: its count is its own again, not the next load's.
      try {
        localStorage.removeItem(DROPPED_KEY)
      } catch {
        // Storage may refuse; the count is a diagnostic.
      }
    }
  }
  globalThis.addEventListener('pagehide', hide)
  globalThis.document?.addEventListener('visibilitychange', visibility)
  schedule()
  return () => {
    if (sender === send) {
      sender = null
    }
    globalThis.removeEventListener('pagehide', hide)
    globalThis.document?.removeEventListener('visibilitychange', visibility)
  }
}

/**
 * Tells the log whether the page is signed in with its password chosen. A
 * page that is not sends nothing and keeps its lines; once it is, it sends them.
 */
export function pageLogSignedIn(signed: boolean): void {
  signedIn = signed
  if (signed && buffer.length + dropped.count > 0) {
    void flush()
  }
}

/**
 * Writes the failure the page found itself as an `error` line, which goes at
 * once; answers the ID of the request that carried it, once it is sent, or null
 * when it did not get in. It says what failed and where, never a value the page held.
 */
export function logPageError(report: PageErrorReport): Promise<string | null> {
  const { conversation, errorClass, requestId, ...fields } = report
  return pageLog.error(ERROR_EVENT, { ...fields, error_class: errorClass, request_id: requestId }, conversation)
}

/**
 * Writes that the browser's storage of `key` could not be read or written, which
 * the page goes on without: the one event of every such failure, told apart by
 * `key` and `op`. `more` names what the key is about, as a device's ID.
 */
export function logStorageFailed(key: string, op: 'read' | 'write', error: unknown, more: Record<string, unknown> = {}): void {
  pageLog.warn('storage.failed', { key, op, ...more, ...errorFields(error) })
}

/** Writes the page's lines. */
export const pageLog = {
  debug: (event: string, fields: Record<string, unknown> = {}, conversationId?: string) =>
    void write('debug', event, fields, conversationId),
  info: (event: string, fields: Record<string, unknown> = {}, conversationId?: string) =>
    void write('info', event, fields, conversationId),
  warn: (event: string, fields: Record<string, unknown> = {}, conversationId?: string) =>
    void write('warn', event, fields, conversationId),
  /** Goes at once; answers the request ID of the request that carried it, or null. */
  error: (event: string, fields: Record<string, unknown> = {}, conversationId?: string) =>
    write('error', event, fields, conversationId),
}

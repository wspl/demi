/**
 * One conversation's `preview` stream as the relay uses it (`preview.md`
 * § The stream): requests, WebSockets and label questions of every tab of
 * the user's browser in that conversation share it, each with an id this
 * side chooses. A request's body moves one chunk per the engine's pull; an
 * answer's comes a window ahead of this side's pulls, each pull sent as a
 * chunk is read, so a small answer takes one round trip. The stream opens with the first
 * request, says `hello` first, and opens again with the next request after
 * it ended: an end, or a frame the protocol refuses, fails every request
 * and socket it carried, as a network error.
 */
import type { OpenUserStream, PreviewPlace, UserStream } from '@demicodes/plugin-sdk'
import {
  PREVIEW_BODY_CHUNK_BYTES,
  type PreviewClient,
  type PageStorage,
  type PreviewEngineMessage,
  type PreviewEnvironment,
  type PreviewHeader,
  type PreviewRelayMessage,
  type PreviewRequest,
} from '../generated/plugin'
import { PreviewFrameReader, encodeMessage, encodeRequestBody, encodeSocketMessage, socketText } from './frames'

/** The head of an answer, and the labels its rewriting computed. */
export interface PreviewHead {
  status: number
  headers: PreviewHeader[]
  labels: Record<string, PreviewEnvironment>
}

/** A request on the stream: its head, then its body, pulled chunk by chunk. */
export interface PreviewExchange {
  /** Rejects with why the request failed: the page sees a network error. */
  readonly head: Promise<PreviewHead>
  /** The body's next chunk; null once it ended. Rejects when the body failed. */
  pull(): Promise<Uint8Array<ArrayBuffer> | null>
  /** The browser gave up on it. */
  cancel(): void
}

/** What a socket of the page hears from upstream. */
export interface PreviewSocketHandlers {
  opened(protocol: string, extensions: string): void
  message(data: string | Uint8Array<ArrayBuffer>): void
  /** The socket closed; 1006 when it failed or never opened. */
  closed(code: number, reason: string): void
}

export interface PreviewSocket {
  send(data: string | Uint8Array): void
  close(code: number, reason: string): void
}

/** What the engine answers a question of the page with. */
type Answer = Extract<PreviewEngineMessage, { type: 'labels' | 'state' | 'state_kept' }>

/** A question waiting for the engine. */
interface Question {
  answered(answer: Answer): void
  reject(error: Error): void
}

/** A page of the agent's browser as a page state moves it into the user's. */
export interface TakenState {
  url: string
  title: string
  /** The agent's tab shows the page in Mobile. */
  mobile: boolean
  /** Its top-level origin's storage; null when it was too large to move. */
  storage: PageStorage | null
  tooLarge: boolean
}

/** Why requests fail when their stream ends. */
export class PreviewStreamEnded extends Error {}

const ABNORMAL_CLOSURE = 1006

interface OpenRequest {
  body: Uint8Array | null
  /** How much of the body was sent. */
  sent: number
  answered(head: PreviewHead): void
  failed(reason: string): void
  /**
   * The reads waiting for their chunks, oldest first: a reader may read
   * again before a chunk arrives, as a stream does while a read waits, and
   * each read gets the next chunk in order.
   */
  chunks: { resolve(data: Uint8Array<ArrayBuffer> | null): void; reject(error: Error): void }[]
  /** Chunks that came ahead of a read, oldest first; null for the body's end. */
  arrived: (Uint8Array<ArrayBuffer> | null)[]
  /** The body ended or failed: no pull waits any more. */
  done: boolean
  /** Why the answer failed after its head, which a later read hears too. */
  broken: Error | null
}

/** The stream of one conversation, opened as the relay needs it. */
export class PreviewConnection {
  private stream: UserStream | null = null
  private reader = new PreviewFrameReader()
  private nextId = 1
  private readonly requests = new Map<number, OpenRequest>()
  private readonly sockets = new Map<number, PreviewSocketHandlers>()
  /** The questions waiting for the engine's answer: labels and page states. */
  private readonly questions = new Map<number, Question>()
  /** The place the open stream said hello with. */
  private helloed: PreviewPlace | null = null

  constructor(private readonly open: OpenUserStream) {}

  /** The stream for `place`, opened and greeted when there is none, or when the place changed. */
  private ready(place: PreviewPlace): UserStream {
    if (this.stream && this.helloed && !samePlace(this.helloed, place)) {
      this.end(new PreviewStreamEnded('where previews live changed'))
    }
    if (this.stream) {
      return this.stream
    }
    this.reader = new PreviewFrameReader()
    const stream = this.open({
      data: (bytes) => {
        if (this.stream !== stream) {
          return
        }
        try {
          for (const frame of this.reader.read(bytes)) {
            if (frame.kind === 'message') {
              this.receive(frame.message)
            } else if (frame.kind === 'chunk') {
              this.chunk(frame.id, frame.data)
            } else {
              // The frame's bytes are the stream's buffer, which the next frame reuses.
              this.sockets.get(frame.id)?.message(frame.binary ? frame.data.slice() : socketText(frame.data))
            }
          }
        } catch (error) {
          // A frame the protocol refuses ends the stream; the next request opens another.
          this.end(new PreviewStreamEnded(error instanceof Error ? error.message : String(error)))
        }
      },
      closed: (reason) => {
        if (this.stream === stream) {
          this.ended(new PreviewStreamEnded(reason || 'the preview stream ended'))
        }
      },
    })
    this.stream = stream
    this.helloed = place
    stream.send(encodeMessage({
      type: 'hello',
      scheme: place.scheme,
      domain: place.domain,
      namespace: place.namespace,
      host: place.host,
    }))
    return stream
  }

  /** Ends the stream this side holds, failing what it carried. */
  private end(error: PreviewStreamEnded): void {
    const stream = this.stream
    this.ended(error)
    stream?.close()
  }

  private ended(error: PreviewStreamEnded): void {
    this.stream = null
    this.helloed = null
    const requests = [...this.requests.values()]
    const sockets = [...this.sockets.values()]
    const questions = [...this.questions.values()]
    this.requests.clear()
    this.sockets.clear()
    this.questions.clear()
    for (const question of questions) {
      question.reject(error)
    }
    for (const request of requests) {
      request.failed(error.message)
      request.broken = error
      request.arrived = []
      for (const waiting of request.chunks.splice(0)) {
        waiting.reject(error)
      }
    }
    for (const socket of sockets) {
      socket.closed(ABNORMAL_CLOSURE, '')
    }
  }

  /** Closes the stream, as the conversation's panel closes. */
  close(): void {
    this.end(new PreviewStreamEnded('the panel closed'))
  }

  private receive(message: PreviewEngineMessage): void {
    switch (message.type) {
      case 'response':
        this.requests.get(message.id)?.answered({ status: message.status, headers: message.headers, labels: message.labels })
        return
      case 'labels':
      case 'state':
      case 'state_kept': {
        const question = this.questions.get(message.id)
        this.questions.delete(message.id)
        question?.answered(message)
        return
      }
      case 'pull': {
        const request = this.requests.get(message.id)
        if (request?.body) {
          const data = request.body.subarray(request.sent, request.sent + PREVIEW_BODY_CHUNK_BYTES)
          request.sent += data.length
          this.stream?.send(encodeRequestBody(message.id, data))
        }
        return
      }
      case 'failed': {
        const question = this.questions.get(message.id)
        if (question) {
          this.questions.delete(message.id)
          question.reject(new Error(message.reason))
          return
        }
        const request = this.requests.get(message.id)
        if (!request) {
          return
        }
        this.requests.delete(message.id)
        request.failed(message.reason)
        request.broken = new Error(message.reason)
        request.arrived = []
        for (const waiting of request.chunks.splice(0)) {
          waiting.reject(new Error(message.reason))
        }
        return
      }
      case 'socket_opened':
        this.sockets.get(message.id)?.opened(message.protocol, message.extensions)
        return
      case 'socket_close': {
        const socket = this.sockets.get(message.id)
        this.sockets.delete(message.id)
        socket?.closed(message.code, message.reason)
        return
      }
    }
  }

  private chunk(id: number, data: Uint8Array): void {
    const request = this.requests.get(id)
    if (!request) {
      return
    }
    // The frame's bytes are the stream's buffer, which the next frame reuses.
    const chunk = data.length === 0 ? null : data.slice()
    const waiting = request.chunks.shift()
    if (!waiting) {
      request.arrived.push(chunk)
      return
    }
    if (chunk === null) {
      this.finished(id, request)
      // The body ended: every read still waiting gets its end.
      for (const read of [waiting, ...request.chunks.splice(0)]) {
        read.resolve(null)
      }
      return
    }
    this.consumed(id)
    waiting.resolve(chunk)
  }

  /** A chunk of request `id`'s answer was read: the engine may send one more. */
  private consumed(id: number): void {
    this.stream?.send(encodeMessage({ type: 'pull', id }))
  }

  /** The answer's body was read to its end. */
  private finished(id: number, request: OpenRequest): void {
    request.done = true
    if (this.requests.get(id) === request) {
      this.requests.delete(id)
    }
  }

  /**
   * Sends one request of a document `environment` received; its initiator
   * and whether the user started it are in `request`.
   */
  request(
    place: PreviewPlace,
    environment: PreviewEnvironment,
    request: Omit<PreviewRequest, 'body'>,
    client: PreviewClient,
    body: Uint8Array | null,
  ): PreviewExchange {
    const stream = this.ready(place)
    const id = this.nextId++
    let answered!: (head: PreviewHead) => void
    let failed!: (reason: string) => void
    const head = new Promise<PreviewHead>((resolve, reject) => {
      answered = resolve
      failed = (reason) => reject(new Error(reason))
    })
    // A request nobody reads the head of still fails quietly.
    head.catch(() => {})
    const open: OpenRequest = { body: body && body.length > 0 ? body : null, sent: 0, answered, failed, chunks: [], arrived: [], done: false, broken: null }
    this.requests.set(id, open)
    stream.send(encodeMessage({ type: 'request', id, environment, request: { ...request, body: open.body !== null }, client }))
    return {
      head,
      pull: () => {
        if (open.broken) {
          return Promise.reject(open.broken)
        }
        if (open.arrived.length > 0) {
          const chunk = open.arrived.shift() ?? null
          if (chunk === null) {
            this.finished(id, open)
          } else {
            this.consumed(id)
          }
          return Promise.resolve(chunk)
        }
        if (open.done || this.requests.get(id) !== open) {
          return Promise.resolve(null)
        }
        return new Promise<Uint8Array<ArrayBuffer> | null>((resolve, reject) => {
          open.chunks.push({ resolve, reject })
        })
      },
      cancel: () => {
        if (this.requests.get(id) !== open) {
          return
        }
        this.requests.delete(id)
        open.done = true
        for (const waiting of open.chunks.splice(0)) {
          waiting.resolve(null)
        }
        this.stream?.send(encodeMessage({ type: 'cancel', id }))
      },
    }
  }

  /**
   * The labels of `environments`, as the engine computes them, each with its
   * environment. Rejects when the stream ends first.
   */
  async labels(place: PreviewPlace, environments: PreviewEnvironment[]): Promise<Record<string, PreviewEnvironment>> {
    const answer = await this.ask(place, (id) => ({ type: 'labels', id, environments }))
    if (answer.type !== 'labels') {
      throw unexpected(answer)
    }
    return answer.labels
  }

  /**
   * The page state of the agent's tab `tab` (`preview.md` § Page state): the
   * engine moved its cookies into the jar, and answers its address, title
   * and top-level origin's storage. Rejects with why it could not.
   */
  async takeState(place: PreviewPlace, tab: string): Promise<TakenState> {
    const answer = await this.ask(place, (id) => ({ type: 'state_take', id, tab }))
    if (answer.type !== 'state') {
      throw unexpected(answer)
    }
    return { url: answer.url, title: answer.title, mobile: answer.mobile, storage: answer.storage, tooLarge: answer.too_large }
  }

  /**
   * Keeps a page state of the user's browser under `token`, for the tab of
   * the agent's browser that opens with it: the jar's cookies of `sites`,
   * and `storage`.
   */
  async keepState(place: PreviewPlace, token: string, sites: string[], storage: PageStorage | null): Promise<void> {
    const answer = await this.ask(place, (id) => ({ type: 'state_keep', id, token, sites, storage }))
    if (answer.type !== 'state_kept') {
      throw unexpected(answer)
    }
  }

  /** Asks the engine `message`; rejects when the stream ends first or the engine fails it. */
  private ask(place: PreviewPlace, message: (id: number) => PreviewRelayMessage): Promise<Answer> {
    const stream = this.ready(place)
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      this.questions.set(id, { answered: resolve, reject })
      stream.send(encodeMessage(message(id)))
    })
  }

  /** Opens a page's WebSocket upstream, from the document `environment` received. */
  socket(
    place: PreviewPlace,
    environment: PreviewEnvironment,
    url: string,
    protocols: string[],
    client: PreviewClient,
    handlers: PreviewSocketHandlers,
  ): PreviewSocket {
    const stream = this.ready(place)
    const id = this.nextId++
    this.sockets.set(id, handlers)
    stream.send(encodeMessage({ type: 'socket_open', id, environment, url, protocols, client }))
    return {
      send: (data) => {
        if (this.sockets.has(id)) {
          this.stream?.send(encodeSocketMessage(id, data))
        }
      },
      close: (code, reason) => {
        if (this.sockets.has(id)) {
          this.stream?.send(encodeMessage({ type: 'socket_close', id, code, reason }))
        }
      },
    }
  }
}

/** The engine answered a question with another's answer, which only a defect of either side does. */
function unexpected(answer: Answer): Error {
  return new Error(`the preview engine answered ${answer.type} to another question`)
}

function samePlace(one: PreviewPlace, other: PreviewPlace): boolean {
  return one.scheme === other.scheme && one.domain === other.domain && one.namespace === other.namespace && one.host === other.host
}

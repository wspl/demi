/**
 * One conversation's `preview` stream as the relay uses it (`preview.md`
 * § The stream): requests, WebSockets and label questions of every tab of
 * the user's browser in that conversation share it, each with an id this
 * side chooses. Bodies
 * move one chunk per pull both ways. The stream opens with the first
 * request, says `hello` first, and opens again with the next request after
 * it ended: an end, or a frame the protocol refuses, fails every request
 * and socket it carried, as a network error.
 */
import type { OpenUserStream, PreviewPlace, UserStream } from '@demicodes/plugin-sdk'
import {
  PREVIEW_BODY_CHUNK_BYTES,
  type PreviewClient,
  type PreviewEngineMessage,
  type PreviewEnvironment,
  type PreviewHeader,
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
   * The pulls waiting for their chunks, oldest first: a reader may pull
   * again before a chunk arrives, as a stream does while a read waits, and
   * each pull gets the next chunk in order.
   */
  chunks: { resolve(data: Uint8Array<ArrayBuffer> | null): void; reject(error: Error): void }[]
  /** The body ended or failed: no pull waits any more. */
  done: boolean
}

/** The stream of one conversation, opened as the relay needs it. */
export class PreviewConnection {
  private stream: UserStream | null = null
  private reader = new PreviewFrameReader()
  private nextId = 1
  private readonly requests = new Map<number, OpenRequest>()
  private readonly sockets = new Map<number, PreviewSocketHandlers>()
  /** The `labels` questions waiting for the engine's answer. */
  private readonly labelQuestions = new Map<number, { resolve(labels: Record<string, PreviewEnvironment>): void; reject(error: Error): void }>()
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
    const questions = [...this.labelQuestions.values()]
    this.requests.clear()
    this.sockets.clear()
    this.labelQuestions.clear()
    for (const question of questions) {
      question.reject(error)
    }
    for (const request of requests) {
      request.failed(error.message)
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
      case 'labels': {
        const question = this.labelQuestions.get(message.id)
        this.labelQuestions.delete(message.id)
        question?.resolve(message.labels)
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
        const request = this.requests.get(message.id)
        if (!request) {
          return
        }
        this.requests.delete(message.id)
        request.failed(message.reason)
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
    const waiting = request?.chunks.shift()
    if (!request || !waiting) {
      return
    }
    if (data.length === 0) {
      request.done = true
      this.requests.delete(id)
      // The body ended: every pull still waiting gets its end.
      for (const pull of [waiting, ...request.chunks.splice(0)]) {
        pull.resolve(null)
      }
    } else {
      // The frame's bytes are the stream's buffer, which the next frame reuses.
      waiting.resolve(data.slice())
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
    const open: OpenRequest = { body: body && body.length > 0 ? body : null, sent: 0, answered, failed, chunks: [], done: false }
    this.requests.set(id, open)
    stream.send(encodeMessage({ type: 'request', id, environment, request: { ...request, body: open.body !== null }, client }))
    return {
      head,
      pull: () => {
        if (open.done || this.requests.get(id) !== open) {
          return Promise.resolve(null)
        }
        return new Promise<Uint8Array<ArrayBuffer> | null>((resolve, reject) => {
          open.chunks.push({ resolve, reject })
          this.stream?.send(encodeMessage({ type: 'pull', id }))
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
  labels(place: PreviewPlace, environments: PreviewEnvironment[]): Promise<Record<string, PreviewEnvironment>> {
    const stream = this.ready(place)
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      this.labelQuestions.set(id, { resolve, reject })
      stream.send(encodeMessage({ type: 'labels', id, environments }))
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

function samePlace(one: PreviewPlace, other: PreviewPlace): boolean {
  return one.scheme === other.scheme && one.domain === other.domain && one.namespace === other.namespace && one.host === other.host
}

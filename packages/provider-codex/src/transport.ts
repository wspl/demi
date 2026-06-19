import { parseSseResponseStream } from './sse'
import type { CodexResponseStreamEvent } from './responses'
import type { CodexTransportMode } from './types'

export interface CodexTransportRequest {
  url: string
  websocketUrl: string
  headers: Headers
  body: unknown
  signal: AbortSignal
  headerTimeoutMs?: number
  websocketConnectTimeoutMs?: number
  streamIdleTimeoutMs?: number
}

export interface CodexResponsesTransport {
  stream(request: CodexTransportRequest): AsyncIterable<CodexResponseStreamEvent>
}

export interface FetchCodexResponsesTransportOptions {
  fetch?: typeof fetch
}

export class FetchCodexResponsesTransport implements CodexResponsesTransport {
  private readonly fetchImpl: typeof fetch

  constructor(options: FetchCodexResponsesTransportOptions = {}) {
    this.fetchImpl = options.fetch ?? fetch
  }

  async *stream(request: CodexTransportRequest): AsyncIterable<CodexResponseStreamEvent> {
    const headerTimeout = createAbortTimeout(request.headerTimeoutMs, 'Codex SSE response headers timed out')
    const combined = combineAbortSignals([request.signal, headerTimeout.signal])
    let response: Response
    try {
      response = await this.fetchImpl(request.url, {
        method: 'POST',
        headers: request.headers,
        body: JSON.stringify(request.body),
        signal: combined.signal,
      })
    } finally {
      headerTimeout.clear()
      combined.cleanup()
    }

    if (!response.ok) {
      throw new CodexHttpError(response.status, response.statusText, await response.text())
    }
    if (!response.body) throw new Error('Codex response did not include a body')

    yield* parseSseResponseStream(response.body)
  }
}

export interface WebSocketCodexResponsesTransportOptions {
  WebSocket?: WebSocketConstructorLike | null
}

export class WebSocketCodexResponsesTransport implements CodexResponsesTransport {
  private readonly WebSocketCtor: WebSocketConstructorLike | null

  constructor(options: WebSocketCodexResponsesTransportOptions = {}) {
    this.WebSocketCtor = options.WebSocket === undefined ? defaultWebSocketConstructor() : options.WebSocket
  }

  async *stream(request: CodexTransportRequest): AsyncIterable<CodexResponseStreamEvent> {
    const socket = await connectWebSocket(
      this.WebSocketCtor,
      request.websocketUrl,
      request.headers,
      request.signal,
      request.websocketConnectTimeoutMs,
    )
    const queue: Array<CodexResponseStreamEvent | Error | null> = []
    const waiters: Array<() => void> = []
    let idleTimer: ReturnType<typeof setTimeout> | null = null
    let finished = false

    const wake = (): void => {
      for (const waiter of waiters.splice(0)) waiter()
    }
    const push = (value: CodexResponseStreamEvent | Error | null): void => {
      queue.push(value)
      wake()
    }
    const finish = (): void => {
      if (finished) return
      finished = true
      push(null)
    }
    const armIdleTimer = (): void => {
      if (!request.streamIdleTimeoutMs) return
      if (idleTimer) clearTimeout(idleTimer)
      idleTimer = setTimeout(() => {
        push(new Error(`Codex WebSocket stream idled for ${request.streamIdleTimeoutMs}ms`))
        socket.close(1000, 'idle_timeout')
      }, request.streamIdleTimeoutMs)
    }
    const cleanup = (): void => {
      if (idleTimer) clearTimeout(idleTimer)
      socket.removeEventListener('message', onMessage)
      socket.removeEventListener('error', onError)
      socket.removeEventListener('close', onClose)
      request.signal.removeEventListener('abort', onAbort)
    }
    const onMessage = (event: MessageEventLike): void => {
      armIdleTimer()
      try {
        const parsed = parseWebSocketMessage(event.data)
        if (parsed) {
          push(parsed)
          if (isTerminalResponseEvent(parsed)) {
            finish()
            socket.close(1000, 'response_done')
          }
        }
      } catch (error) {
        push(error instanceof Error ? error : new Error(String(error)))
      }
    }
    const onError = (): void => push(new Error('Codex WebSocket error'))
    const onClose = (): void => finish()
    const onAbort = (): void => {
      socket.close(1000, 'aborted')
      push(new DOMException('Aborted', 'AbortError'))
    }

    socket.addEventListener('message', onMessage)
    socket.addEventListener('error', onError)
    socket.addEventListener('close', onClose)
    request.signal.addEventListener('abort', onAbort, { once: true })

    try {
      socket.send(JSON.stringify({ type: 'response.create', ...(request.body as Record<string, unknown>) }))
      armIdleTimer()
      while (true) {
        if (queue.length === 0) {
          await new Promise<void>((resolve) => waiters.push(resolve))
        }
        const next = queue.shift()
        if (next === undefined) continue
        if (next === null) return
        if (next instanceof Error) throw next
        yield next
      }
    } finally {
      cleanup()
      socket.close(1000, 'done')
    }
  }
}

export class AutoCodexResponsesTransport implements CodexResponsesTransport {
  constructor(
    private readonly websocket: CodexResponsesTransport,
    private readonly sse: CodexResponsesTransport,
  ) {}

  async *stream(request: CodexTransportRequest): AsyncIterable<CodexResponseStreamEvent> {
    let started = false
    try {
      for await (const event of this.websocket.stream(request)) {
        started = true
        yield event
      }
      return
    } catch (error) {
      if (started) throw error
    }
    yield* this.sse.stream(request)
  }
}

export class CodexHttpError extends Error {
  constructor(
    readonly status: number,
    statusText: string,
    readonly responseText: string,
  ) {
    super(codexHttpErrorMessage(status, statusText, responseText))
    this.name = 'CodexHttpError'
  }
}

export function codexResponsesUrl(baseUrl: string): string {
  const normalized = baseUrl.replace(/\/+$/, '')
  if (normalized.endsWith('/responses')) return normalized
  if (normalized.endsWith('/codex')) return `${normalized}/responses`
  return `${normalized}/codex/responses`
}

export function codexWebSocketUrl(responsesUrl: string): string {
  return responsesUrl.replace(/^https:/, 'wss:').replace(/^http:/, 'ws:')
}

export function createCodexTransport(mode: CodexTransportMode, options: FetchCodexResponsesTransportOptions & WebSocketCodexResponsesTransportOptions = {}): CodexResponsesTransport {
  const sse = new FetchCodexResponsesTransport(options)
  if (mode === 'sse') return sse
  const websocket = new WebSocketCodexResponsesTransport(options)
  if (mode === 'websocket') return websocket
  return new AutoCodexResponsesTransport(websocket, sse)
}

interface WebSocketConstructorLike {
  new (url: string, options?: { headers?: Record<string, string> }): WebSocketLike
}

interface WebSocketLike {
  send(data: string): void
  close(code?: number, reason?: string): void
  addEventListener(type: 'open' | 'message' | 'error' | 'close', listener: (event: never) => void): void
  removeEventListener(type: 'open' | 'message' | 'error' | 'close', listener: (event: never) => void): void
}

interface MessageEventLike {
  data: unknown
}

function createAbortTimeout(ms: number | undefined, message: string): { signal: AbortSignal; clear(): void } {
  const controller = new AbortController()
  if (!ms || ms <= 0) return { signal: controller.signal, clear: () => undefined }
  const timeout = setTimeout(() => controller.abort(new Error(`${message} after ${ms}ms`)), ms)
  return { signal: controller.signal, clear: () => clearTimeout(timeout) }
}

function combineAbortSignals(signals: AbortSignal[]): { signal: AbortSignal; cleanup(): void } {
  const controller = new AbortController()
  const listeners: Array<() => void> = []
  for (const signal of signals) {
    const listener = (): void => controller.abort(signal.reason)
    listeners.push(() => signal.removeEventListener('abort', listener))
    if (signal.aborted) controller.abort(signal.reason)
    else signal.addEventListener('abort', listener, { once: true })
  }
  return {
    signal: controller.signal,
    cleanup: () => {
      for (const cleanup of listeners) cleanup()
    },
  }
}

function connectWebSocket(
  WebSocketCtor: WebSocketConstructorLike | null,
  url: string,
  headers: Headers,
  signal: AbortSignal,
  timeoutMs = 10_000,
): Promise<WebSocketLike> {
  if (!WebSocketCtor) return Promise.reject(new Error('WebSocket transport is not available in this runtime'))
  const headerRecord = headersToRecord(headers)
  delete headerRecord.accept
  delete headerRecord['content-type']
  headerRecord['openai-beta'] = 'responses_websockets=2026-02-06'

  return new Promise((resolve, reject) => {
    let socket: WebSocketLike
    let settled = false
    const timeout = setTimeout(() => fail(new Error(`Codex WebSocket connect timed out after ${timeoutMs}ms`)), timeoutMs)
    const cleanup = (): void => {
      clearTimeout(timeout)
      socket?.removeEventListener('open', onOpen)
      socket?.removeEventListener('error', onError)
      socket?.removeEventListener('close', onClose)
      signal.removeEventListener('abort', onAbort)
    }
    const fail = (error: Error): void => {
      if (settled) return
      settled = true
      cleanup()
      try {
        socket?.close(1000, 'connect_failed')
      } catch {
        // ignore close failures
      }
      reject(error)
    }
    const onOpen = (): void => {
      if (settled) return
      settled = true
      cleanup()
      resolve(socket)
    }
    const onError = (): void => fail(new Error('Codex WebSocket connect error'))
    const onClose = (): void => fail(new Error('Codex WebSocket closed before opening'))
    const onAbort = (): void => fail(new DOMException('Aborted', 'AbortError'))

    try {
      socket = new WebSocketCtor(url, { headers: headerRecord })
    } catch (error) {
      clearTimeout(timeout)
      reject(error instanceof Error ? error : new Error(String(error)))
      return
    }

    socket.addEventListener('open', onOpen)
    socket.addEventListener('error', onError)
    socket.addEventListener('close', onClose)
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

function parseWebSocketMessage(data: unknown): CodexResponseStreamEvent | null {
  if (typeof data === 'string') return parseWebSocketJson(data)
  if (data instanceof ArrayBuffer) return parseWebSocketJson(new TextDecoder().decode(data))
  if (data instanceof Uint8Array) return parseWebSocketJson(new TextDecoder().decode(data))
  return null
}

function parseWebSocketJson(text: string): CodexResponseStreamEvent | null {
  const parsed = JSON.parse(text) as CodexResponseStreamEvent | { type?: string; event?: CodexResponseStreamEvent; response?: unknown }
  if (parsed.type === 'response.done') {
    return { type: 'response.completed', response: parsed.response as CodexResponseStreamEvent['response'] }
  }
  if ('event' in parsed && isRecord(parsed.event)) return parsed.event as CodexResponseStreamEvent
  return parsed as CodexResponseStreamEvent
}

function isTerminalResponseEvent(event: CodexResponseStreamEvent): boolean {
  return event.type === 'response.completed' || event.type === 'response.failed' || event.type === 'response.incomplete' || event.type === 'error'
}

function headersToRecord(headers: Headers): Record<string, string> {
  const out: Record<string, string> = {}
  headers.forEach((value, key) => {
    out[key] = value
  })
  return out
}

function codexHttpErrorMessage(status: number, statusText: string, responseText: string): string {
  const body = parseJsonObject(responseText)
  const error = isRecord(body?.error) ? body.error : null
  return typeof error?.message === 'string' ? error.message : responseText || statusText || `HTTP ${status}`
}

function parseJsonObject(text: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(text)
    return isRecord(parsed) ? parsed : null
  } catch {
    return null
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function defaultWebSocketConstructor(): WebSocketConstructorLike | null {
  return typeof globalThis.WebSocket === 'function'
    ? (globalThis.WebSocket as unknown as WebSocketConstructorLike)
    : null
}

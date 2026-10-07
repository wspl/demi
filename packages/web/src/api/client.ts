import { z } from 'zod'
import { errorBodySchema, type ErrorCode } from './generated/web-api'

/**
 * A request the backend refused, with its `{ code, message }`; `code` is null
 * when something in front of the backend answered instead.
 */
export class ApiError extends Error {
  readonly status: number
  readonly code: ErrorCode | null
  /** A plugin's own word for its refusal, with `plugin_refused`. */
  readonly reason: string | null
  /** How many seconds the answer's `Retry-After` asks to wait, as a locked sign-in's does. */
  readonly retryAfterSeconds: number | null

  constructor(
    status: number,
    code: ErrorCode | null,
    message: string,
    reason: string | null = null,
    retryAfterSeconds: number | null = null,
  ) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.reason = reason
    this.retryAfterSeconds = retryAfterSeconds
  }
}

let expired: (() => void) | null = null

export function onSessionExpired(handler: () => void): () => void {
  expired = handler
  return () => {
    if (expired === handler) {
      expired = null
    }
  }
}

export function notifySessionExpired(code: ErrorCode): void {
  if (code === 'unauthenticated') {
    notifySessionEnded()
  }
}

/** Ends the session on the page: an answer of 401 does, and so does the synchronization channel's `session_ended`. */
export function notifySessionEnded(): void {
  expired?.()
}

export async function readResponse<T>(
  response: Response,
  schema: z.ZodType<T>,
): Promise<T> {
  const parsed = schema.safeParse(await response.json())
  if (!parsed.success) {
    throw invalidResponse(parsed.error)
  }
  return parsed.data
}

/** An answer, body or headers, that does not match its schema. */
export function invalidResponse(error: z.ZodError): Error {
  return new Error(`Invalid server response: ${error.issues[0]?.message ?? 'unknown shape'}`)
}

/** What a proxy in front of the backend answers while no backend runs behind it, as while it restarts. */
const AWAY_STATUSES = new Set([502, 503, 504])

/**
 * Whether a request failed because the backend could not be reached, not
 * because it answered: no answer came (no network, no connection, no answer
 * in time), a proxy in front of the backend said it is away, or the backend
 * said it is shutting down and did not do the request (`backend.md`
 * § Startup and shutdown). Every other answer of the backend's own carries its
 * `{ code, message }` body, so an error with another code is the backend's
 * answer, whatever its status.
 */
function unreachable(error: unknown): boolean {
  if (error instanceof TypeError) {
    // What fetch rejects with when no connection could be made.
    return true
  }
  if (error instanceof DOMException && error.name === 'TimeoutError') {
    return true
  }
  if (!(error instanceof ApiError)) {
    return false
  }
  return error.code === null ? AWAY_STATUSES.has(error.status) : error.code === 'backend_closing'
}

/** The URL of an API path, for what the web browser loads itself: an image, a player, a download. */
export function apiUrl(path: string): string {
  return `/api${path}`
}

/** How long a request may take unless it says otherwise. */
const REQUEST_TIMEOUT_MS = 60_000

/**
 * Resolves once the backend is worth asking again after `attempt` tries that
 * could not reach it; rejects when `signal` aborts first, or when the page
 * stops following its state.
 */
export type BackendWait = (attempt: number, signal: AbortSignal) => Promise<void>

/** How a request waits for the backend; none while the page follows no state, as before sign-in. */
let backendWait: BackendWait | null = null

/**
 * Makes every request wait with `wait` while it cannot reach the backend, as
 * the product state does while it follows the backend's synchronization
 * channel, which decides when the backend is reached again. Returns the
 * undo.
 */
export function waitForBackendWith(wait: BackendWait): () => void {
  backendWait = wait
  return () => {
    if (backendWait === wait) {
      backendWait = null
    }
  }
}

/** A signal that never aborts, for a request whose caller gives none. */
const NEVER = new AbortController().signal

interface ApiRequestOptions extends RequestInit {
  allowNotModified?: boolean
  /** For a request whose work is known to take longer than most, such as starting the conversation browser on a Cloud. */
  timeoutMs?: number
  /**
   * False for a request that must not wait for the backend: one by which the
   * page learns whether it still reaches the backend, as the
   * synchronization channel's own question after it could not connect.
   */
  waits?: boolean
}

/**
 * Sends with `send` until the backend answers (`web-application.md` § A page
 * of another build): a try that cannot reach the backend, as `unreachable`
 * judges it, waits until the page reaches the backend again and goes then,
 * so no caller sees a failure about the connection. Only the backend's own
 * answer rejects. Once `signal` aborts, it rejects with the abort's reason
 * and sends nothing more. Every request of the page goes through it: the
 * HTTP client's, and an upload's bytes.
 */
export async function untilReached<T>(send: () => Promise<T>, signal?: AbortSignal): Promise<T> {
  for (let attempt = 1; ; attempt += 1) {
    try {
      return await send()
    } catch (error) {
      if (signal?.aborted) {
        throw signal.reason
      }
      const wait = backendWait
      if (!wait || !unreachable(error)) {
        throw error
      }
      await wait(attempt, signal ?? NEVER)
    }
  }
}

/**
 * Sends a request to the backend; same-origin requests use the backend's
 * HttpOnly session cookie. A read or a write alike waits while it cannot
 * reach the backend (`untilReached`), unless `waits` is false.
 */
export async function apiRequest(
  path: string,
  options: ApiRequestOptions = {},
): Promise<Response> {
  const { waits = true, ...request } = options
  return waits ? untilReached(() => send(path, request), request.signal ?? undefined) : send(path, request)
}

/** One try of a request, with its own deadline. */
async function send(path: string, options: Omit<ApiRequestOptions, 'waits'>): Promise<Response> {
  const { allowNotModified, timeoutMs, ...request } = options
  const timeout = AbortSignal.timeout(timeoutMs ?? REQUEST_TIMEOUT_MS)
  const signal = options.signal
    ? AbortSignal.any([options.signal, timeout])
    : timeout
  const response = await fetch(apiUrl(path), {
    ...request,
    credentials: 'same-origin',
    cache: 'no-store',
    signal,
  })
  if (response.status === 204) {
    // Chrome counts a request whose answer the page never reads as aborted
    // (net::ERR_ABORTED), though the backend answered it, and most callers
    // of an answer without content never read it. It is read to its end
    // here, and the caller gets the same answer with nothing left to read.
    await response.arrayBuffer()
    return new Response(null, { status: 204, statusText: response.statusText, headers: response.headers })
  }
  if (response.ok || (allowNotModified && response.status === 304)) {
    return response
  }

  throw apiError(response.status, await response.text(), response.headers.get('Retry-After'))
}

/**
 * The error a failed answer stands for: the backend's `{ code, message }`,
 * or its status alone when something in front of the backend answered. An
 * expired session is noticed on the way.
 */
/** `Retry-After` in seconds; its date form is not one the backend sends. */
const retryAfterSchema = z.coerce.number().int().nonnegative()

export function apiError(status: number, text: string, retryAfter: string | null = null): ApiError {
  let body: unknown
  try {
    body = JSON.parse(text)
  } catch {
    // A reverse proxy can return HTML instead of the backend error contract.
    body = null
  }
  const parsed = errorBodySchema.safeParse(body)
  if (parsed.success) {
    notifySessionExpired(parsed.data.code)
  }
  return new ApiError(
    status,
    parsed.success ? parsed.data.code : null,
    parsed.success ? parsed.data.message : `Request failed (${status}).`,
    parsed.success ? parsed.data.reason ?? null : null,
    retryAfter === null ? null : retryAfterSchema.safeParse(retryAfter).data ?? null,
  )
}

export function jsonBody(value: unknown): RequestInit {
  return {
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(value),
  }
}

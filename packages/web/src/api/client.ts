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

  constructor(status: number, code: ErrorCode | null, message: string, reason: string | null = null) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
    this.reason = reason
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

/**
 * Whether a request failed because the backend could not be reached, not
 * because it refused: no network, no answer in time, or a proxy in front of
 * the backend saying it is away.
 */
export function unreachable(error: unknown): boolean {
  if (error instanceof TypeError) {
    // What fetch rejects with when no connection could be made.
    return true
  }
  if (error instanceof DOMException && error.name === 'TimeoutError') {
    return true
  }
  // The backend answers every error with its body; a bare server error is
  // something in front of it, such as a proxy with no backend behind it.
  return error instanceof ApiError && error.code === null && error.status >= 500
}

/** The URL of an API path, for what the web browser loads itself: an image, a player, a download. */
export function apiUrl(path: string): string {
  return `/api${path}`
}

/** How long a request may take unless it says otherwise. */
const REQUEST_TIMEOUT_MS = 60_000

interface ApiRequestOptions extends RequestInit {
  allowNotModified?: boolean
  /** For a request whose work is known to take longer than most, such as starting the conversation browser on a Cloud. */
  timeoutMs?: number
}

/** Same-origin requests use the backend's HttpOnly session cookie. */
export async function apiRequest(
  path: string,
  options: ApiRequestOptions = {},
): Promise<Response> {
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
  if (response.ok || (allowNotModified && response.status === 304)) {
    return response
  }

  throw apiError(response.status, await response.text())
}

/**
 * The error a failed answer stands for: the backend's `{ code, message }`,
 * or its status alone when something in front of the backend answered. An
 * expired session is noticed on the way.
 */
export function apiError(status: number, text: string): ApiError {
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
  )
}

export function jsonBody(value: unknown): RequestInit {
  return {
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(value),
  }
}

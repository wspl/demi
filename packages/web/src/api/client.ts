import { z } from 'zod'

const errorSchema = z.object({
  code: z.string(),
  message: z.string(),
})

export class ApiError extends Error {
  readonly status: number
  readonly code: string

  constructor(status: number, code: string, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
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

export function notifySessionExpired(code: string): void {
  if (code === 'unauthenticated') {
    expired?.()
  }
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

/** The URL of an API path, for what the browser loads itself: an image, a player, a download. */
export function apiUrl(path: string): string {
  return `/api${path}`
}

/** How long a request may take unless it says otherwise. */
const REQUEST_TIMEOUT_MS = 60_000

interface ApiRequestOptions extends RequestInit {
  allowNotModified?: boolean
  /** For a request whose work is known to take longer than most, such as starting a browser on a Cloud. */
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
  const parsed = errorSchema.safeParse(body)
  if (parsed.success) {
    notifySessionExpired(parsed.data.code)
  }
  return new ApiError(
    status,
    parsed.success ? parsed.data.code : 'http_error',
    parsed.success ? parsed.data.message : `Request failed (${status})`,
  )
}

export function jsonBody(value: unknown): RequestInit {
  return {
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(value),
  }
}

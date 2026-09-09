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
    throw new Error(
      `Invalid server response: ${parsed.error.issues[0]?.message ?? 'unknown shape'}`,
    )
  }
  return parsed.data
}

interface ApiRequestOptions extends RequestInit {
  allowNotModified?: boolean
}

/** Same-origin requests use the backend's HttpOnly session cookie. */
export async function apiRequest(
  path: string,
  options: ApiRequestOptions = {},
): Promise<Response> {
  const { allowNotModified, ...request } = options
  const timeout = AbortSignal.timeout(60_000)
  const signal = options.signal
    ? AbortSignal.any([options.signal, timeout])
    : timeout
  const response = await fetch(`/api${path}`, {
    ...request,
    credentials: 'same-origin',
    cache: 'no-store',
    signal,
  })
  if (response.ok || (allowNotModified && response.status === 304)) {
    return response
  }

  const text = await response.text()
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
  throw new ApiError(
    response.status,
    parsed.success ? parsed.data.code : 'http_error',
    parsed.success ? parsed.data.message : `Request failed (${response.status})`,
  )
}

export function jsonBody(value: unknown): RequestInit {
  return {
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(value),
  }
}

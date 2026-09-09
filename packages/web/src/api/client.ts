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

/** Same-origin requests use the backend's HttpOnly session cookie. */
export async function apiRequest(
  path: string,
  options: RequestInit = {},
): Promise<Response> {
  const timeout = AbortSignal.timeout(15_000)
  const signal = options.signal
    ? AbortSignal.any([options.signal, timeout])
    : timeout
  const response = await fetch(`/api${path}`, {
    ...options,
    credentials: 'same-origin',
    cache: 'no-store',
    signal,
  })
  if (response.ok)
    return response

  const text = await response.text()
  let body: unknown
  try {
    body = JSON.parse(text)
  } catch {
    // A reverse proxy can return HTML instead of the backend error contract.
    body = null
  }
  const parsed = errorSchema.safeParse(body)
  throw new ApiError(
    response.status,
    parsed.success ? parsed.data.code : 'http_error',
    parsed.success
      ? parsed.data.message
      : `Request failed (${response.status})`,
  )
}

export function jsonBody(value: unknown): RequestInit {
  return {
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(value),
  }
}

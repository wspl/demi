/** Error thrown when an operation is canceled via an `AbortSignal` or a session abort. */
export class AbortError extends Error {
  constructor(message = 'Aborted') {
    super(message)
    this.name = 'AbortError'
  }
}

/** Normalizes an unknown thrown value into an `Error`. */
export function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error))
}

/** Extracts a human-readable message from an unknown thrown value, falling back to `String(error)`
 * for non-Errors and for Errors with an empty message. */
export function errorMessage(error: unknown): string {
  return error instanceof Error && error.message.length > 0 ? error.message : String(error)
}

/** Reports whether an unknown thrown value represents an abort: the {@link AbortError} class, a
 * `DOMException` from a fetch/AbortController abort, or any object with `name === 'AbortError'`. */
export function isAbortError(error: unknown): boolean {
  if (error instanceof AbortError) return true
  return typeof error === 'object' && error !== null && (error as { name?: unknown }).name === 'AbortError'
}

/** Throws an `AbortError` if the signal is already aborted. */
export function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw new AbortError()
}

/**
 * Wraps a promise so it rejects with `AbortError` as soon as the signal aborts. The underlying
 * promise is not canceled; this only stops the caller from awaiting it past the abort.
 */
export function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(new AbortError())
  return new Promise<T>((resolve, reject) => {
    let settled = false
    const onAbort = (): void => {
      if (settled) return
      settled = true
      signal.removeEventListener('abort', onAbort)
      reject(new AbortError())
    }
    signal.addEventListener('abort', onAbort, { once: true })
    promise.then(
      (value) => {
        if (settled) return
        settled = true
        signal.removeEventListener('abort', onAbort)
        resolve(value)
      },
      (error) => {
        if (settled) return
        settled = true
        signal.removeEventListener('abort', onAbort)
        reject(error)
      },
    )
  })
}

/** Reports whether an unknown thrown value is a file-not-found error (ENOENT). */
export function isFileNotFoundError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    (error as { code?: unknown }).code === 'ENOENT'
  )
}

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

/** Extracts a human-readable message from an unknown thrown value. */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** Reports whether an unknown thrown value represents an abort. */
export function isAbortError(error: unknown): boolean {
  return error instanceof AbortError || (error instanceof Error && error.name === 'AbortError')
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

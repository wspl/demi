/** Whether `error` is the rejection of work an `AbortSignal` stopped. */
export function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError'
}

/** Normalizes an unknown thrown value into an `Error`. */
export function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error))
}

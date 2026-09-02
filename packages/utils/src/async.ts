/** A no-op function. */
export function noop(): void {}

/** An externally-resolvable promise handle. */
export interface Deferred<T> {
  readonly promise: Promise<T>
  resolve(value: T | PromiseLike<T>): void
  reject(reason?: unknown): void
}

/** Creates a promise whose `resolve`/`reject` are exposed for external settlement. */
export function deferred<T = void>(): Deferred<T> {
  let resolve!: Deferred<T>['resolve']
  let reject!: Deferred<T>['reject']
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

/**
 * Resolves after `ms` milliseconds. With `signal`, resolves as soon as the
 * signal aborts instead, and the timer is cleared — for a `Promise.race`
 * whose loser must not keep the process alive.
 */
export function delay(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal?.aborted) return resolve()
    const timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort)
      resolve()
    }, ms)
    function onAbort() {
      clearTimeout(timer)
      resolve()
    }
    signal?.addEventListener('abort', onAbort, { once: true })
  })
}

/**
 * Polls `predicate` until it returns `true`, or rejects once `timeoutMs` elapses.
 * `describe` (optional) supplies extra context for the timeout error.
 */
export async function waitFor(
  predicate: () => boolean,
  describe?: () => string,
  options: { timeoutMs?: number; intervalMs?: number } = {},
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? 1_000
  const intervalMs = options.intervalMs ?? 1
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > timeoutMs) {
      const detail = describe?.().trim()
      throw new Error(`Timed out waiting for condition${detail ? `: ${detail}` : ''}`)
    }
    await new Promise((resolve) => setTimeout(resolve, intervalMs))
  }
}

/** Rejects with `Error(message)` if `promise` does not settle within `ms` milliseconds. */
export function withTimeout<T>(promise: Promise<T>, ms: number, message = `Timed out after ${ms}ms`): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), ms)
    promise.then(
      (value) => {
        clearTimeout(timer)
        resolve(value)
      },
      (error) => {
        clearTimeout(timer)
        reject(error)
      },
    )
  })
}

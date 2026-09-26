/** Orders operations on one resource without blocking unrelated resources. */
export class SerialQueue {
  private tail: Promise<unknown> = Promise.resolve()
  private pending = 0

  get idle(): boolean {
    return this.pending === 0
  }

  run<T>(operation: () => Promise<T>): Promise<T> {
    this.pending += 1
    const next = this.tail.then(operation).finally(() => {
      this.pending -= 1
    })
    // The caller receives a failure through `next`; the tail only keeps the
    // order, so the next operation runs after a failed one too.
    this.tail = next.catch(() => {})
    return next
  }

  async settled(): Promise<void> {
    await this.tail
  }
}

/** An externally-resolvable promise handle. */
export interface Deferred<T> {
  readonly promise: Promise<T>
  resolve(value: T | PromiseLike<T>): void
  reject(reason?: unknown): void
}

/**
 * Creates a promise whose `resolve`/`reject` are exposed for external
 * settlement.
 */
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
    if (signal?.aborted)
      return resolve()
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
  options: {
    timeoutMs?: number;
    intervalMs?: number
  } = {},
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? 1_000
  const intervalMs = options.intervalMs ?? 1
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > timeoutMs) {
      const detail = describe?.().trim()
      throw new Error(
        `Timed out waiting for condition${detail ? `: ${detail}` : ''}`
      )
    }
    await new Promise((resolve) => setTimeout(resolve, intervalMs))
  }
}

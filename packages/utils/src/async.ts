/**
 * Orders operations by the things they change: the operations on one thing
 * run one at a time, in the order they were added, and a failed one does not
 * stop the next; operations on different things run side by side. An
 * operation on several things waits for the earlier operations on each of
 * them, and the later operations on any of them wait for it.
 */
export class SerialQueue {
  /** The end of the last operation added on each thing, while it runs or waits. */
  private readonly tails = new Map<string, Promise<void>>()

  run<T>(keys: string | readonly string[], operation: () => Promise<T>): Promise<T> {
    const things = typeof keys === 'string' ? [keys] : [...new Set(keys)]
    const earlier = things.flatMap((key) => this.tails.get(key) ?? [])
    const next = Promise.all(earlier).then(operation)
    // The caller receives a failure through `next`; a tail only keeps the
    // order, so the next operation runs after a failed one too.
    const tail = next.then(() => {}, () => {})
    for (const key of things) {
      this.tails.set(key, tail)
    }
    void tail.then(() => {
      for (const key of things) {
        if (this.tails.get(key) === tail) {
          this.tails.delete(key)
        }
      }
    })
    return next
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

/**
 * Runs work by key one run at a time: a call for a key whose run is under
 * way shares that run's promise instead of starting another, and the key is
 * free again once the run settles, whether it succeeded or failed.
 */
export function oneRunPerKey<K, T>(): (key: K, work: () => Promise<T>) => Promise<T> {
  const running = new Map<K, Promise<T>>()
  return (key, work) => {
    const pending = running.get(key)
    if (pending) {
      return pending
    }
    const run = work().finally(() => running.delete(key))
    running.set(key, run)
    return run
  }
}

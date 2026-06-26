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

/** Resolves after `ms` milliseconds. */
export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
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

import { deferred } from './async'
import { abortable, throwIfAborted } from './errors'

/** Concurrent operations with an exclusive, cancellable admission barrier. */
export class ActivityGate {
  private readers = 0
  private reserved = false
  private changed = deferred<void>()

  get active(): boolean { return this.readers > 0 }

  async enter(signal?: AbortSignal): Promise<() => void> {
    while (this.reserved) await this.wait(signal)
    if (signal) throwIfAborted(signal)
    this.readers++
    return this.once(() => { this.readers--; this.notify() })
  }

  /** Reserves only an idle gate, synchronously with respect to new entrants. */
  tryReserve(): (() => void) | null {
    if (this.reserved || this.readers > 0) return null
    this.reserved = true
    return this.once(() => { this.reserved = false; this.notify() })
  }

  /** Stops new entrants, then waits for admitted operations to finish. */
  async reserve(signal?: AbortSignal): Promise<() => void> {
    while (this.reserved) await this.wait(signal)
    if (signal) throwIfAborted(signal)
    this.reserved = true
    const release = this.once(() => { this.reserved = false; this.notify() })
    try {
      while (this.readers > 0) await this.wait(signal)
      return release
    } catch (error) { release(); throw error }
  }

  private wait(signal?: AbortSignal): Promise<void> {
    return signal ? abortable(this.changed.promise, signal) : this.changed.promise
  }

  private notify(): void {
    const changed = this.changed
    this.changed = deferred<void>()
    changed.resolve()
  }

  private once(action: () => void): () => void {
    let done = false
    return () => { if (!done) { done = true; action() } }
  }
}

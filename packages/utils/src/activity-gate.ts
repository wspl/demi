import { deferred } from './async'
import { abortable, throwIfAborted } from './errors'

export type ActivityReservationPurpose = 'idle' | 'forced'

/** Concurrent operations with an exclusive, cancellable admission barrier. */
export class ActivityGate {
  private readonly leases = new Map<() => void, 'demand' | 'maintenance'>()
  private reservation: { release: () => void; purpose: ActivityReservationPurpose } | undefined
  private readonly listeners = new Set<() => void>()
  private changed = deferred<void>()

  /** Idle retirement can be waited through; forced transitions refuse new work. */
  get reservationPurpose(): ActivityReservationPurpose | undefined {
    return this.reservation?.purpose
  }

  get active(): boolean {
    return this.leases.size > 0
  }

  get demandActive(): boolean {
    return [...this.leases.values()].includes('demand')
  }

  /** Observe ordered admission changes without acquiring activity. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  /** Verify an existing exclusive reservation before borrowing it for cleanup. */
  holdsReservation(release: () => void): boolean {
    return this.reservation?.release === release
  }

  async enter(signal?: AbortSignal, purpose: 'demand' | 'maintenance' = 'demand'): Promise<() => void> {
    while (this.reservation) await this.wait(signal)
    if (signal)
      throwIfAborted(signal)
    return this.admit(purpose)
  }

  /**
   * Admits immediately, or refuses while an exclusive transition owns the gate.
   */
  tryEnter(purpose: 'demand' | 'maintenance' = 'demand'): (() => void) | null {
    if (this.reservation)
      return null
    return this.admit(purpose)
  }

  /** Reserves only an idle gate, synchronously with respect to new entrants. */
  tryReserve(purpose: ActivityReservationPurpose): (() => void) | null {
    if (this.reservation || this.active)
      return null
    return this.claim(purpose)
  }

  /** Stops new entrants, then waits for admitted operations to finish. */
  async reserve(purpose: ActivityReservationPurpose, signal?: AbortSignal): Promise<() => void> {
    while (this.reservation) await this.wait(signal)
    if (signal)
      throwIfAborted(signal)
    const release = this.claim(purpose)
    try {
      while (this.active) await this.wait(signal)
      return release
    } catch (error) {
      release();
      throw error
    }
  }

  private wait(signal?: AbortSignal): Promise<void> {
    return signal
      ? abortable(this.changed.promise, signal)
      : this.changed.promise
  }

  private notify(): void {
    const changed = this.changed
    this.changed = deferred<void>()
    changed.resolve()
    for (const listener of this.listeners) {
      try {
        listener()
      } catch (error) {
        // Observers cannot roll back admission or prevent other leases releasing.
        console.error('Activity observer failed', error)
      }
    }
  }

  private admit(purpose: 'demand' | 'maintenance'): () => void {
    const release = () => {
      if (!this.leases.delete(release)) return
      this.notify()
    }
    this.leases.set(release, purpose)
    this.notify()
    return release
  }

  private claim(purpose: ActivityReservationPurpose): () => void {
    const release = () => {
      if (this.reservation?.release !== release) return
      this.reservation = undefined
      this.notify()
    }
    this.reservation = { release, purpose }
    this.notify()
    return release
  }
}

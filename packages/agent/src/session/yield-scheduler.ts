interface PendingYieldWakeup<Metadata> {
  id: string
  durationMs: number
  timer: ReturnType<typeof setTimeout> | null
  dueAt: number | null
  armed: boolean
  metadata: Metadata
}

/**
 * Tracks scheduled `yield` wakeups and their timers. This class owns only the
 * registry and timer lifecycle; the owner decides what a fired wakeup actually
 * does (interject as a steer, or start a fresh turn) via the `onFire` callback.
 */
export class YieldScheduler<Metadata = undefined> {
  private readonly pending: PendingYieldWakeup<Metadata>[] = []

  constructor(
    private readonly idFactory: () => string,
    private readonly onFire: (wakeupId: string, metadata: Metadata) => void,
  ) {}

  get hasPending(): boolean {
    return this.pending.length > 0
  }

  /** Registers a new (unarmed) wakeup and returns its id. */
  schedule(durationMs: number, metadata: Metadata): string {
    const id = this.idFactory()
    this.pending.push({ id, durationMs, timer: null, dueAt: null, armed: false, metadata })
    return id
  }

  /** Arms timers for any not-yet-armed wakeups; each fires `onFire(id)` when due. */
  arm(): void {
    const now = Date.now()
    for (const wakeup of this.pending) {
      if (wakeup.armed) continue
      wakeup.armed = true
      wakeup.dueAt = now + wakeup.durationMs
      wakeup.timer = setTimeout(() => this.onFire(wakeup.id, wakeup.metadata), wakeup.durationMs)
    }
  }

  /** Removes the wakeup with `wakeupId` (clearing its timer); returns whether it existed. */
  take(wakeupId: string): boolean {
    const index = this.pending.findIndex((wakeup) => wakeup.id === wakeupId)
    if (index === -1) return false
    const [wakeup] = this.pending.splice(index, 1)
    if (wakeup?.timer) clearTimeout(wakeup.timer)
    return true
  }

  /** Cancels the oldest pending wakeup, if any; returns whether one was cancelled. */
  cancelOne(): boolean {
    const wakeup = this.pending.shift()
    if (!wakeup) return false
    if (wakeup.timer) clearTimeout(wakeup.timer)
    return true
  }

  /** Cancels every pending wakeup. */
  clear(): void {
    for (const wakeup of this.pending.splice(0)) {
      if (wakeup.timer) clearTimeout(wakeup.timer)
    }
  }
}

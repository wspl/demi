import type { ModelSelection, UserContentBlock } from '@demicodes/core'

export interface PendingSteer {
  id: string
  turnId: string
  model: ModelSelection
  content: UserContentBlock[]
  hidden?: boolean
}

/**
 * Bookkeeping for steers awaiting materialization: the pending list, the set of
 * steers canceled before delivery, and a monotonic continuation counter the turn
 * loop snapshots to detect steers that arrived mid-stream. Pure state — the
 * session owns the delivery and materialization decisions.
 */
export class PendingSteerQueue {
  private readonly pending: PendingSteer[] = []
  private readonly canceledIds = new Set<string>()
  private continuation = 0

  /** Monotonic count of steers ever enqueued (decremented only when a pending one is removed). */
  get continuationCount(): number {
    return this.continuation
  }

  /** Enqueues a steer for later materialization. */
  add(steer: PendingSteer): void {
    this.pending.push(steer)
    this.continuation += 1
  }

  /** Removes a still-pending steer by id; returns whether one was removed. */
  removePending(id: string): boolean {
    const index = this.pending.findIndex((steer) => steer.id === id)
    if (index === -1) return false
    this.pending.splice(index, 1)
    this.continuation = Math.max(0, this.continuation - 1)
    return true
  }

  /** Records that a not-yet-delivered steer should be dropped when it arrives. */
  markCanceled(id: string): void {
    this.canceledIds.add(id)
  }

  /** Consumes a recorded cancellation; returns whether `id` had been canceled. */
  takeCanceled(id: string): boolean {
    return this.canceledIds.delete(id)
  }

  /** Forgets all recorded cancellations. */
  clearCanceled(): void {
    this.canceledIds.clear()
  }

  /** Removes and returns every pending steer for `turnId`, preserving order. */
  takeForTurn(turnId: string): PendingSteer[] {
    const steers: PendingSteer[] = []
    for (let index = 0; index < this.pending.length; ) {
      const steer = this.pending[index]
      if (steer.turnId !== turnId) {
        index += 1
        continue
      }
      steers.push(steer)
      this.pending.splice(index, 1)
    }
    return steers
  }
}

import type { AgentMessage, ModelSelection, PendingSteer } from '@demicodes/core'
import type { AgentMetadata } from '../types'

export interface PendingInternalSteer {
  turnId: string
  model: ModelSelection
  agentMessage: AgentMessage
  metadata: AgentMetadata | null
}

type QueuedSteer = (PendingSteer & { hidden?: boolean; agentMessage?: never }) | PendingInternalSteer

function steerId(steer: QueuedSteer): string {
  return steer.agentMessage ? steer.agentMessage.id : steer.id
}

/**
 * Bookkeeping for steers awaiting materialization: the pending list, the set of
 * steers canceled before delivery, and a monotonic continuation counter the
 * turn
 * loop snapshots to detect steers that arrived mid-stream. Pure state — the
 * session owns the delivery and materialization decisions.
 */
export class PendingSteerQueue {
  private readonly pending: QueuedSteer[] = []
  private readonly canceledIds = new Set<string>()
  private continuation = 0

  internalSnapshot(): PendingInternalSteer[] {
    return structuredClone(this.pending.filter((steer): steer is PendingInternalSteer => Boolean(steer.agentMessage)))
  }

  has(id: string): boolean {
    return this.pending.some((steer) => steerId(steer) === id)
  }

  containsInternal(id: string): boolean {
    return this.pending.some((steer) => steer.agentMessage?.id === id)
  }

  get hasInternal(): boolean {
    return this.pending.some((steer) => steer.agentMessage)
  }

  retargetInternal(turnId: string): void {
    for (const steer of this.pending) {
      if (steer.agentMessage) {
        steer.turnId = turnId
      }
    }
  }

  /** Detached user-facing data; internal wakeups never leave the session. */
  snapshot(): PendingSteer[] {
    return structuredClone(this.pending.filter((steer): steer is PendingSteer => !steer.agentMessage && !steer.hidden).map((
      { id, turnId, model, content }
    ) => ({
      id, turnId, model, content,
    })))
  }

  /**
   * Monotonic count of steers ever enqueued (decremented only when a pending
   * one is removed).
   */
  get continuationCount(): number {
    return this.continuation
  }

  /** Enqueues a steer for later materialization. */
  add(steer: QueuedSteer): void {
    this.pending.push(steer)
    this.continuation += 1
  }

  /** Removes a still-pending steer by id; returns whether one was removed. */
  removePending(id: string, internal = false): boolean {
    const index = this.pending.findIndex((steer) => steerId(steer) === id && (internal || !steer.agentMessage))
    if (index === -1)
      return false
    this.pending.splice(index, 1)
    this.continuation = Math.max(0, this.continuation - 1)
    return true
  }

  /**
   * Records that a not-yet-delivered steer should be dropped when it arrives.
   */
  markCanceled(id: string): void {
    this.canceledIds.add(id)
  }

  /**
   * Consumes a recorded cancellation; returns whether `id` had been canceled.
   */
  takeCanceled(id: string): boolean {
    return this.canceledIds.delete(id)
  }

  /** Forgets all recorded cancellations. */
  clearCanceled(): void {
    this.canceledIds.clear()
  }

  /** Removes and returns every pending steer for `turnId`, preserving order. */
  takeForTurn(turnId: string, includeInternal = true): QueuedSteer[] {
    const steers: QueuedSteer[] = []
    for (let index = 0; index < this.pending.length; ) {
      const steer = this.pending[index]
      if (steer.turnId !== turnId || (!includeInternal && steer.agentMessage)) {
        index += 1
        continue
      }
      steers.push(steer)
      this.pending.splice(index, 1)
    }
    return steers
  }
}

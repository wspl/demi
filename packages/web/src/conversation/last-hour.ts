import { watch } from 'vue'
import { isAbortError } from '@demicodes/utils'
import type { LastHourCost } from '../api/generated/web-api'
import type { Conversation } from '../state/types'

/** How often a watched last hour is read again with no new call. */
export const LAST_HOUR_READ_MS = 60_000

/** What a watch of the last hour needs of the product. */
export interface LastHourSource {
  read(conversationId: string, signal: AbortSignal): Promise<LastHourCost>
  /** Reports a failed read; never called for one that was ended. */
  failed(error: unknown): void
}

/**
 * Keeps the last hour of a conversation's cost current while a card shows it
 * (`docs/decisions/0041-the-cost-shows-the-last-hour.md`): the backend counts
 * it, so the page reads it when the first card starts to show it, after each
 * change of the conversation's cost, and once a minute, and drops it when the
 * last card stops. The cards of a conversation, its composer's and its
 * agents' windows', share one watch.
 *
 * ```text
 *  start ──► read ──► conversation.lastHour = answer
 *              ▲
 *   a call, or a minute
 * ```
 *
 * An answer that a read started after it has already replaced is dropped.
 */
export function createLastHourWatch(source: LastHourSource) {
  const watches = new Map<Conversation, { cards: number, end: () => void }>()

  function begin(conversation: Conversation): () => void {
    const controller = new AbortController()
    let started = 0
    let taken = 0
    const read = () => {
      const mine = ++started
      source.read(conversation.id, controller.signal).then(
        (answer) => {
          if (mine > taken) {
            taken = mine
            conversation.lastHour = answer
          }
        },
        (error: unknown) => {
          if (!isAbortError(error) && !controller.signal.aborted)
            source.failed(error)
        },
      )
    }
    read()
    const timer = setInterval(read, LAST_HOUR_READ_MS)
    // Each agent's cost changes with its calls, and the rows of no agent with
    // the conversation's metadata. The costs are compared by value, so what
    // else of an agent or of the metadata changes reads nothing.
    const stopFollowing = watch(
      () => JSON.stringify([conversation.cost, conversation.subagents.map((agent) => agent.cost), conversation.metadata.shown.value?.otherCost]),
      read,
    )
    return () => {
      clearInterval(timer)
      stopFollowing()
      controller.abort()
    }
  }

  /** Starts showing the conversation's last hour; the function it answers stops. */
  return function watchLastHour(conversation: Conversation): () => void {
    let watching = watches.get(conversation)
    if (!watching) {
      watching = { cards: 0, end: begin(conversation) }
      watches.set(conversation, watching)
    }
    const shared = watching
    shared.cards += 1
    let stopped = false
    return () => {
      if (stopped)
        return
      stopped = true
      shared.cards -= 1
      if (shared.cards === 0) {
        shared.end()
        watches.delete(conversation)
      }
    }
  }
}

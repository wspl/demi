import { watch } from 'vue'
import { isAbortError } from '@demicodes/utils'
import type { CostWindow, RecentCost } from '../api/generated/web-api'
import type { Conversation } from '../state/types'

/** How often a watched window is read again with no new call. */
export const RECENT_COST_READ_MS = 60_000

/** What a watch of a window needs of the product. */
export interface RecentCostSource {
  read(conversationId: string, window: CostWindow, signal: AbortSignal): Promise<RecentCost>
  /** Reports a failed read; never called for one that was ended. */
  failed(error: unknown): void
}

/**
 * Keeps a window of a conversation's cost, the last hour or the last day,
 * current while a card shows it: the backend counts it, so the page reads it
 * when the first card starts to show it, after each change of the
 * conversation's cost, and once a minute, and drops the watch when the last
 * card stops. The cards of a conversation that show the same window, its
 * composer's and its agents' windows', share one watch.
 *
 * ```text
 *  start ──► read ──► conversation.recent[window] = answer
 *              ▲
 *   a call, or a minute
 * ```
 *
 * An answer that a read started after it has already replaced is dropped.
 */
export function createRecentCostWatch(source: RecentCostSource) {
  const watches = new Map<Conversation, Partial<Record<CostWindow, { cards: number, end: () => void }>>>()

  function begin(conversation: Conversation, window: CostWindow): () => void {
    const controller = new AbortController()
    let started = 0
    let taken = 0
    const read = () => {
      const mine = ++started
      source.read(conversation.id, window, controller.signal).then(
        (answer) => {
          if (mine > taken) {
            taken = mine
            conversation.recent[window] = answer
          }
        },
        (error: unknown) => {
          if (!isAbortError(error) && !controller.signal.aborted)
            source.failed(error)
        },
      )
    }
    read()
    const timer = setInterval(read, RECENT_COST_READ_MS)
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

  /** Starts showing the conversation's `window`; the function it answers stops. */
  return function watchRecentCost(conversation: Conversation, window: CostWindow): () => void {
    const windows = watches.get(conversation) ?? {}
    watches.set(conversation, windows)
    const watching = windows[window] ?? { cards: 0, end: begin(conversation, window) }
    windows[window] = watching
    watching.cards += 1
    let stopped = false
    return () => {
      if (stopped)
        return
      stopped = true
      watching.cards -= 1
      if (watching.cards === 0) {
        watching.end()
        delete windows[window]
        if (Object.keys(windows).length === 0)
          watches.delete(conversation)
      }
    }
  }
}

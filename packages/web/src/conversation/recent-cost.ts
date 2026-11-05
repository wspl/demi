import { watch } from 'vue'
import { isAbortError } from '@demicodes/utils'
import { NO_COST, agentCost, conversationCost, recentOf, type CostShown } from '@demicodes/web-ui/agent/context-cost'
import type { CostWindow, RecentCost } from '../api/generated/web-api'
import type { Conversation } from '../state/types'

/**
 * What the context card of the conversation, `node` null, or of its agent
 * `node`, shows of their cost: from their agents' metadata, with the cache
 * chart of the node it shows, and the last hour's and the last day's cost
 * while a card shows them (`createRecentCostWatch`).
 */
export function costShown(conversation: Conversation, node: string | null = null): CostShown | null {
  const chart = conversation.charts[node ?? conversation.id] ?? null
  const firstCallAt = conversation.metadata.shown.value?.firstCallAt ?? null
  const recent = (number: number | null) => {
    const of = (window: CostWindow) => {
      const answer = conversation.recent[window]
      return answer ? recentOf(answer.agents, conversation.subagents, number) : null
    }
    return { hour: of('hour'), day: of('day') }
  }
  if (node === null) {
    const other = conversation.metadata.shown.value?.otherCost ?? NO_COST
    return { cost: conversationCost(conversation.cost, conversation.subagents, other), node: null, chart, recent: recent(null), firstCallAt }
  }
  const agent = conversation.subagents.find((each) => each.id === node)
  return agent ? { cost: agentCost(agent.cost), node, chart, recent: recent(agent.number), firstCallAt } : null
}

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

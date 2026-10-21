import type { AgentCost, CacheChart, CostPrice, CostTotals } from '@demicodes/protocol'
import { t } from './locales/text'

/**
 * What a conversation, or one of its agents, cost, as the context card shows
 * it (`packages/web-ui/src/agent/ContextUsageIndicator.vue`): the totals the
 * backend keeps with each agent's metadata, summed for the conversation, and
 * each subagent's price, which the Profiles part sums by profile.
 */
export interface ConversationCostView {
  totals: CostTotals
  /** The calls its totals count: the agent's own, or every agent's for the conversation. */
  calls: number
  /** The main conversation's price and each subagent's; none for one agent's card. */
  agents: AgentPrice[]
}

/** One agent's price: the main conversation's, agent 0, or a subagent's, with the profile it was started with. */
export interface AgentPrice {
  agent: number
  profile: string | null
  price: number
}

/**
 * What a card's Cost section shows: the cost, and the cache chart of the
 * agent it shows, which the backend counts and sends with the agent's node
 * (`docs/decisions/0019-the-server-counts-the-cache-chart.md`).
 */
export interface CostShown {
  cost: ConversationCostView
  /** Null until the agent's node first brought it. */
  chart: CacheChart | null
}

const NO_PRICE: CostPrice = { total: null, input: null, cacheWrite: null, cacheRead: null, output: null }

/** The cost of an agent or a conversation that has no ledger row yet. */
export const NO_COST: AgentCost = {
  totals: { inputTokens: 0, cacheWriteTokens: 0, cacheReadTokens: 0, outputTokens: 0, price: NO_PRICE },
  calls: 0,
}

/** A sum of prices that may have none: null while no part has one. */
function addPrice(a: number | null, b: number | null): number | null {
  return a === null ? b : b === null ? a : a + b
}

function addPrices(a: CostPrice, b: CostPrice): CostPrice {
  return {
    total: addPrice(a.total, b.total),
    input: addPrice(a.input, b.input),
    cacheWrite: addPrice(a.cacheWrite, b.cacheWrite),
    cacheRead: addPrice(a.cacheRead, b.cacheRead),
    output: addPrice(a.output, b.output),
  }
}

/** What `costs` cost together. */
export function sumCosts(costs: readonly AgentCost[]): AgentCost {
  return costs.reduce<AgentCost>((sum, cost) => ({
    totals: {
      inputTokens: sum.totals.inputTokens + cost.totals.inputTokens,
      cacheWriteTokens: sum.totals.cacheWriteTokens + cost.totals.cacheWriteTokens,
      cacheReadTokens: sum.totals.cacheReadTokens + cost.totals.cacheReadTokens,
      outputTokens: sum.totals.outputTokens + cost.totals.outputTokens,
      price: addPrices(sum.totals.price, cost.totals.price),
    },
    calls: sum.calls + cost.calls,
  }), NO_COST)
}

/**
 * A conversation's cost from its agents' metadata: the main conversation's
 * own `root`, each subagent's in `subagents`, and the rows of no agent,
 * `other`, such as its titles'.
 */
export function conversationCost(
  root: AgentCost,
  subagents: readonly { number: number, profile: string | null, cost: AgentCost }[],
  other: AgentCost,
): ConversationCostView {
  const whole = sumCosts([root, ...subagents.map((agent) => agent.cost), other])
  const priced = (agent: number, profile: string | null, cost: AgentCost): AgentPrice[] =>
    cost.totals.price.total === null ? [] : [{ agent, profile, price: cost.totals.price.total }]
  return {
    totals: whole.totals,
    calls: whole.calls,
    agents: [
      ...priced(0, null, root),
      ...subagents.flatMap((agent) => priced(agent.number, agent.profile, agent.cost)),
    ],
  }
}

/** One agent's own cost, as its window's card shows it. */
export function agentCost(cost: AgentCost): ConversationCostView {
  return { totals: cost.totals, calls: cost.calls, agents: [] }
}

/**
 * Whether the conversation made any call: a conversation with none has no Cost
 * section.
 */
export function hasCost(cost: ConversationCostView | null): cost is ConversationCostView {
  if (cost === null)
    return false
  const { totals } = cost
  return cost.calls > 0
    || totals.price.total !== null
    || totals.inputTokens + totals.cacheWriteTokens + totals.cacheReadTokens + totals.outputTokens > 0
}

/** One row of the Profiles part. */
export interface ProfileRow {
  key: string
  label: string
  price: number
}

/** The profiles the Profiles part shows at most, those that cost the most. */
export const PROFILE_ROWS = 5

/**
 * The Profiles part's rows: the main conversation first, for scale, then the
 * sum of each enabled profile's subagents, the `PROFILE_ROWS` largest, largest
 * first. A profile beyond them has no row. A subagent started
 * with no profile, or with one not in `enabled` (since disabled or deleted),
 * has no row. With no profile row there is no part, and no row.
 */
export function profileRows(agents: readonly AgentPrice[], enabled: readonly string[]): ProfileRow[] {
  const sums = new Map<string, number>()
  for (const agent of agents) {
    if (agent.agent !== 0 && agent.profile !== null && enabled.includes(agent.profile))
      sums.set(agent.profile, (sums.get(agent.profile) ?? 0) + agent.price)
  }
  if (sums.size === 0)
    return []
  const main = agents.find((agent) => agent.agent === 0)
  const profiles = [...sums]
    .map(([profile, price]) => ({ key: `profile:${profile}`, label: profile, price }))
    .sort((a, b) => b.price - a.price)
    .slice(0, PROFILE_ROWS)
  return [...(main ? [{ key: 'main', label: t('context.cost.mainConversation'), price: main.price }] : []), ...profiles]
}

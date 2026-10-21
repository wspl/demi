import { useLocalStorage } from '@vueuse/core'
import { inject, provide, type InjectionKey, type Ref } from 'vue'
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
 * (`docs/decisions/0019-the-server-counts-the-cache-chart.md`), and what the
 * same agents cost in the last hour, which the backend counts when a card
 * that shows it asks
 * (`docs/decisions/0041-the-cost-shows-the-last-hour.md`).
 */
export interface CostShown {
  cost: ConversationCostView
  /** Null until the agent's node first brought it. */
  chart: CacheChart | null
  /** What the last read of the last hour answered; null until the first read of it answers. */
  lastHour: LastHourShown | null
  /** When the conversation's first call ended, as an ISO time; null before it made one. */
  firstCallAt: string | null
}

/** What the card's agents cost in the last hour, and the cache's share of the shown agent's prompts. */
export interface LastHourShown {
  cost: ConversationCostView
  /** Null without a turn call that sent a prompt. */
  rate: number | null
}

/** How far back the Cost section counts: the last 60 minutes, or the whole conversation. */
export type CostRange = 'lastHour' | 'all'

/** What the backend counts of one agent's last hour; null for the rows of no agent. */
export interface LastHourEntry {
  agent: number | null
  cost: AgentCost
  rate: number | null
}

const HOUR_MS = 60 * 60 * 1000

/**
 * Whether the card offers the last hour: from the time the conversation's
 * first call is more than an hour old, before which both ranges would show
 * the same.
 */
export function lastHourOffered(firstCallAt: string | null, now: number = Date.now()): boolean {
  return firstCallAt !== null && now - Date.parse(firstCallAt) > HOUR_MS
}

/** Where the browser keeps the range the Cost section counts, for every conversation. */
const COST_RANGE_KEY = 'demi-cost-range'

/** The range the Cost section counts, All until the user chooses. */
export function useCostRange(): Ref<CostRange> {
  return useLocalStorage<CostRange>(COST_RANGE_KEY, 'all', {
    serializer: { read: (raw) => (raw === 'lastHour' ? 'lastHour' : 'all'), write: (range) => range },
  })
}

/**
 * Starts a watch of the conversation's last hour, which the page keeps
 * current while it runs, and answers the function that ends it. A card that
 * shows the last hour holds one for as long as it shows it.
 */
export type CostWatch = () => () => void

const costWatchKey: InjectionKey<CostWatch> = Symbol('cost-watch')

/** Gives the cards below the page's way of keeping the last hour current. */
export function provideCostWatch(watch: CostWatch): void {
  provide(costWatchKey, watch)
}

/** The watch a card starts while it shows the last hour; none where nothing counts it. */
export function useCostWatch(): CostWatch | undefined {
  return inject(costWatchKey, undefined)
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
 * What the last hour's `entries` come to for the conversation's card, `node`
 * null, or for the agent numbered `node`: an agent without an entry made no
 * call in the hour.
 */
export function lastHourOf(
  entries: readonly LastHourEntry[],
  subagents: readonly { number: number, profile: string | null }[],
  node: number | null,
): LastHourShown {
  const of = (agent: number | null) => entries.find((entry) => entry.agent === agent)
  const costOf = (agent: number | null) => of(agent)?.cost ?? NO_COST
  if (node !== null)
    return { cost: agentCost(costOf(node)), rate: of(node)?.rate ?? null }
  const view = conversationCost(
    costOf(0),
    subagents.map((agent) => ({ number: agent.number, profile: agent.profile, cost: costOf(agent.number) })),
    costOf(null),
  )
  return { cost: view, rate: of(0)?.rate ?? null }
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

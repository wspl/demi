import { ref } from 'vue'
import type { CacheMiss, CostCall } from '@demicodes/protocol'
import type { AgentPrice, ConversationCostView, CostShown } from '@demicodes/web-ui/agent/context-cost'
import { type GlobalProfiles, workflowProfiles } from '@demicodes/web-ui/agent/workflow-menu'
import { demoSubagentProfiles } from './subagent-profiles'
import { MINUTE } from './time'

/** A deterministic sequence in [0, 1), so every load draws the same conversation. */
function sequence(seed: number): () => number {
  let state = seed
  return () => {
    state = (state * 1_664_525 + 1_013_904_223) >>> 0
    return state / 4_294_967_296
  }
}

/** Claude Opus's prices, dollars per million tokens. */
const RATE = { input: 5, cacheWrite: 6.25, cacheRead: 0.5, output: 25 }

/**
 * The calls a compaction comes before, and the misses with their causes;
 * past the first 180, a giant conversation compacts every 105 calls and
 * misses every 97th.
 */
const COMPACTED_BEFORE = new Set([31, 68, 131])
const MISSES = new Map<number, CacheMiss>([
  [7, 'not_cached'],
  [75, 'cache_expired'],
  [137, 'system_prompt_changed'],
])
const LATER_COMPACTIONS = 105
const LATER_MISSES = 97
function compactedBefore(number: number): boolean {
  return COMPACTED_BEFORE.has(number) || (number > 180 && number % LATER_COMPACTIONS === 0)
}
function missAt(number: number): CacheMiss | null {
  return MISSES.get(number) ?? (number > 180 && number % LATER_MISSES === 0 ? 'not_cached' : null)
}
/** How long before the miss its pause was: past the vendor's five minutes. */
const PAUSE_BEFORE = new Map([[75, 9 * MINUTE]])

/**
 * The user's subagent settings as Settings › Subagents shows them, Subagents
 * on; archivist is disabled.
 */
export const GLOBAL_PROFILES: GlobalProfiles = { enabled: true, profiles: demoSubagentProfiles() }

/** The profiles Global Workflow offers the card specimens. */
export const OFFERED_PROFILES: readonly string[] = workflowProfiles({ type: 'default' }, { state: 'nowhere' }, GLOBAL_PROFILES)

/** Eight profiles a workflow offers, more than the Profiles part shows. */
export const MANY_PROFILES: readonly string[] = ['explore', 'reviewer', 'scout', 'planner', 'tester', 'writer', 'translator', 'designer']

/**
 * A subagent's price as the fixture states it: with profiles, one of them
 * since disabled, which has no row; with each of `MANY_PROFILES`; or without
 * any.
 */
const SUBAGENTS: Record<'profiles' | 'many' | 'plain', AgentPrice[]> = {
  profiles: [
    { agent: 1, profile: 'explore', price: 1.32 },
    { agent: 2, profile: 'explore', price: 1.32 },
    { agent: 3, profile: 'reviewer', price: 1.32 },
    { agent: 4, profile: 'archivist', price: 0.88 },
    { agent: 5, profile: null, price: 0.56 },
  ],
  many: MANY_PROFILES.map((profile, index) => ({ agent: index + 1, profile, price: 0.24 + ((index * 5) % 8) * 0.31 })),
  plain: [
    { agent: 1, profile: null, price: 1.32 },
    { agent: 2, profile: null, price: 1.32 },
    { agent: 3, profile: null, price: 0.88 },
  ],
}

/**
 * A long conversation's cost: 180 turn calls of the main conversation over
 * three hours and a half, three compactions, three misses of three causes, and
 * subagents started with profiles, without them, or none; or the first
 * `turns` of it, a conversation just started, or more, a giant one; with
 * the recent calls the card reads as it opens, as the backend answers them.
 */
export function costFixture(subagents: keyof typeof SUBAGENTS | 'none', turns = 180, priced = true): { cost: ConversationCostView, calls: CostCall[] } {
  const random = sequence(7)
  const calls: CostCall[] = []
  let prompt = 0
  // A giant conversation started long enough ago for its calls to end by now.
  let at = Date.now() - Math.max(212, turns * 1.25) * MINUTE
  for (let number = 1; number <= turns; number++) {
    at += PAUSE_BEFORE.get(number) ?? (40_000 + random() * 50_000)
    const fresh = number === 1 || compactedBefore(number)
    const miss = missAt(number)
    prompt = fresh ? 24_000 + random() * 12_000 : prompt + 600 + random() * 1_400
    const input = Math.round(40 + random() * 400)
    const cacheWrite = Math.round(fresh || miss ? prompt * 0.97 : 400 + random() * 2_000)
    const cacheRead = Math.max(0, Math.round(prompt - cacheWrite - input))
    const output = Math.round(300 + random() * 2_500)
    const price = (input * RATE.input + cacheWrite * RATE.cacheWrite + cacheRead * RATE.cacheRead + output * RATE.output) / 1e6
    calls.push({
      number,
      startedAt: new Date(at).toISOString(),
      inputTokens: input,
      cacheWriteTokens: cacheWrite,
      cacheReadTokens: cacheRead,
      price,
      afterCompaction: compactedBefore(number),
      miss,
    })
  }
  const sum = (pick: (call: CostCall) => number) => calls.reduce((total, call) => total + pick(call), 0)
  const main = sum((call) => call.price ?? 0)
  const others = subagents === 'none' ? [] : SUBAGENTS[subagents]
  const elsewhere = others.reduce((total, agent) => total + agent.price, 0)
  // The subagents' tokens, priced as the main conversation's split, a fifth of theirs each way.
  const share = 1 + elsewhere / main
  const input = sum((call) => call.inputTokens) * share
  const cacheWrite = sum((call) => call.cacheWriteTokens) * share
  const cacheRead = sum((call) => call.cacheReadTokens) * share
  const total = main + elsewhere
  const kinds = input * RATE.input + cacheWrite * RATE.cacheWrite + cacheRead * RATE.cacheRead
  const output = Math.max(0, (total * 1e6 - kinds) / RATE.output)
  if (!priced) {
    // A model without catalog prices, as a compatible entry's: counts only.
    for (const call of calls)
      call.price = null
  }
  const unpriced = { total: null, input: null, cacheWrite: null, cacheRead: null, output: null }
  const cost: ConversationCostView = {
    calls: calls.length + others.length,
    totals: {
      inputTokens: Math.round(input),
      cacheWriteTokens: Math.round(cacheWrite),
      cacheReadTokens: Math.round(cacheRead),
      outputTokens: Math.round(output),
      price: !priced ? unpriced : {
        total,
        input: (input * RATE.input) / 1e6,
        cacheWrite: (cacheWrite * RATE.cacheWrite) / 1e6,
        cacheRead: (cacheRead * RATE.cacheRead) / 1e6,
        output: (output * RATE.output) / 1e6,
      },
    },
    agents: priced ? [{ agent: 0, profile: null, price: main }, ...others] : [],
  }
  // The backend answers the recent 400, as the chart draws them.
  return { cost, calls: calls.slice(-400) }
}

/** The request id a specimen's failed read names, as the backend's look. */
const FAILED_REQUEST = 'req_7f3a9c2e41d84b06'

/**
 * What a card specimen holds of its conversation's cost, as the product's
 * store does: the cost from the agents' metadata, and the calls the card
 * reads while it is open, `calls` its opening and closing. A `failing`
 * specimen's read of the calls failed, and Retry reads them, as the
 * product's would once the backend answers.
 */
export function useCardCost(subagents: keyof typeof SUBAGENTS | 'none', { failing = false, turns = 180, priced = true } = {}) {
  const fixture = costFixture(subagents, turns, priced)
  const read: CostShown['calls'] = { status: 'read', calls: fixture.calls }
  const cost = ref<CostShown>({
    cost: fixture.cost,
    calls: failing ? { status: 'failed', error: { message: 'The backend could not be reached.', requestId: FAILED_REQUEST } } : read,
  })
  function retry(): void {
    cost.value = { ...cost.value, calls: read }
  }
  /** The card opened or closed: an opened one holds the calls read, or the failed read, again. */
  function calls(wanted: boolean): void {
    if (wanted && cost.value.calls === null)
      cost.value = { ...cost.value, calls: failing ? { status: 'failed', error: { message: 'The backend could not be reached.', requestId: FAILED_REQUEST } } : read }
    else if (!wanted)
      cost.value = { ...cost.value, calls: null }
  }
  return { cost, retry, calls }
}

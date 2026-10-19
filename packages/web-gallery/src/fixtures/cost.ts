import { ref } from 'vue'
import type { AgentCost, CacheMiss, CostCall } from '@demicodes/protocol'
import type { ConversationCostView, CostState } from '@demicodes/web-ui/agent/context-cost'
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
const COMPACTED_BEFORE = new Set([68, 131])
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

/**
 * A subagent's price as the fixture states it: with profiles, one of them
 * since disabled, which has no row; or without any.
 */
const SUBAGENTS: Record<'profiles' | 'plain', AgentCost[]> = {
  profiles: [
    { agent: 1, profile: 'explore', price: 1.32 },
    { agent: 2, profile: 'explore', price: 1.32 },
    { agent: 3, profile: 'reviewer', price: 1.32 },
    { agent: 4, profile: 'archivist', price: 0.88 },
    { agent: 5, profile: null, price: 0.56 },
  ],
  plain: [
    { agent: 1, profile: null, price: 1.32 },
    { agent: 2, profile: null, price: 1.32 },
    { agent: 3, profile: null, price: 0.88 },
  ],
}

/**
 * A long conversation's cost: 180 turn calls of the main conversation over
 * three hours and a half, two compactions, three misses of three causes, and
 * subagents started with profiles, without them, or none; or the first
 * `turns` of it, a conversation just started, or more, a giant one.
 */
export function costFixture(subagents: 'profiles' | 'plain' | 'none', turns = 180): ConversationCostView {
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
  return {
    rows: calls.length + others.length,
    totals: {
      inputTokens: Math.round(input),
      cacheWriteTokens: Math.round(cacheWrite),
      cacheReadTokens: Math.round(cacheRead),
      outputTokens: Math.round(output),
      price: {
        total,
        input: (input * RATE.input) / 1e6,
        cacheWrite: (cacheWrite * RATE.cacheWrite) / 1e6,
        cacheRead: (cacheRead * RATE.cacheRead) / 1e6,
        output: (output * RATE.output) / 1e6,
      },
    },
    agents: [{ agent: 0, profile: null, price: main }, ...others],
    calls,
  }
}

/** The request id a specimen's failed read names, as the backend's look. */
const FAILED_REQUEST = 'req_7f3a9c2e41d84b06'

/**
 * What a card specimen holds of its conversation's cost, as the product's
 * store does: the cost read with the conversation, there before the card
 * opens. A `failing` specimen's read failed, and Retry reads it, as the
 * product's would once the backend answers.
 */
export function useCardCost(subagents: 'profiles' | 'plain' | 'none', { failing = false, turns = 180 } = {}) {
  const cost = ref<CostState>(failing
    ? { status: 'failed', error: { message: 'The backend could not be reached.', requestId: FAILED_REQUEST } }
    : { status: 'read', cost: costFixture(subagents, turns) })
  function retry(): void {
    cost.value = { status: 'read', cost: costFixture(subagents, turns) }
  }
  return { cost, retry }
}

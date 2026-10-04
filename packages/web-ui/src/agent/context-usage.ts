import type { ContextUsage } from '@demicodes/protocol'

/**
 * A share of the window as the page shows it: whole percent, rounded down,
 * as the backend words its refusal (`compaction.md` § When compaction runs).
 */
function percentOf(tokens: number, window: number): number {
  return Math.floor((tokens * 100) / window)
}

/** How full the context is, in whole percent of its window; null without a window. */
export function contextPercent(usage: ContextUsage | null): number | null {
  if (!usage?.window) {
    return null
  }
  return Math.min(percentOf(usage.tokens, usage.window), 100)
}

/**
 * Why the user may not compact at `usage`, or null when they may: below the
 * estimate the backend takes a `compact` frame from. A model without a
 * window, and a usage not known yet, leave the decision to the backend.
 */
export function compactionRefusal(usage: ContextUsage | null): string | null {
  if (!usage?.window || usage.compactFrom == null || usage.tokens >= usage.compactFrom) {
    return null
  }
  const from = percentOf(usage.compactFrom, usage.window)
  return `Compaction is available from ${from}% context usage (now ${percentOf(usage.tokens, usage.window)}%)`
}

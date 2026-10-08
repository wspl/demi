import type { ContextUsage } from '@demicodes/protocol'
import { formatPercent } from '../ui/percent'

/** How full the context is, from 0 to 1 of its window, full at most; null without a window. */
export function contextRatio(usage: ContextUsage | null): number | null {
  if (!usage?.window) {
    return null
  }
  return Math.min(usage.tokens / usage.window, 1)
}

/**
 * How full the context is as the page says it: whole percent of its
 * window, rounded down as every meter's label is (`formatPercent`) and as
 * the backend words its refusal (`compaction.md` § When compaction runs);
 * null without a window.
 */
export function contextPercent(usage: ContextUsage | null): string | null {
  return usage?.window ? formatPercent(usage.tokens, usage.window) : null
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
  const from = formatPercent(usage.compactFrom, usage.window)
  return `Compaction is available from ${from} context usage (now ${formatPercent(usage.tokens, usage.window)})`
}

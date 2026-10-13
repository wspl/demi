import type { Ref } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { ContextRowKind, ContextUsage } from '@demicodes/protocol'
import type { SettingsQuotaWindow } from '../settings/types'
import { formatPercent } from '../ui/percent'
import { formatTokens } from '../ui/token-count'

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

/** Why Compact waits while the agent's turn runs, which the session refuses as busy. */
export const COMPACT_AFTER_TURN = 'Compaction is available once the turn ends.'

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

/** What a row of the context card is called (`compaction.md` § What fills the context). */
const ROW_LABELS: Record<ContextRowKind, string> = {
  messages: 'Messages',
  systemPrompt: 'System prompt',
  commands: 'Commands',
  instructions: 'Instructions',
  skills: 'Skills',
  otherContext: 'Other context',
  summary: 'Summary',
  compactionBuffer: 'Compaction buffer',
  freeSpace: 'Free space',
}

/** One row of the context card, one figure. */
export interface ContextCardRow {
  kind: ContextRowKind
  label: string
  tokens: number
  estimated: boolean
}

/**
 * What the context card's header counts: the rows before the compaction
 * buffer, which add up to the estimate, or to more when the rows above
 * Messages alone exceed it; the estimate itself while the usage has no rows.
 */
export function contextFilled(usage: ContextUsage): number {
  const filled = usage.rows
    .filter((row) => row.kind !== 'compactionBuffer' && row.kind !== 'freeSpace')
    .reduce((sum, row) => sum + row.tokens, 0)
  return Math.max(usage.tokens, filled)
}

/**
 * The figure the context card's header shows and the ring's tip repeats:
 * what fills the context against the window and its share, such as
 * `139K / 1M (13%)`; the filled tokens alone while the model has no window.
 */
export function contextFigure(usage: ContextUsage): string {
  const filled = formatTokens(contextFilled(usage))
  return usage.window ? `${filled} / ${formatTokens(usage.window)} (${contextPercent(usage)})` : filled
}

/** The rows that stand after what the request carries, in this order. */
const TRAILING_ROWS: readonly ContextRowKind[] = ['compactionBuffer', 'freeSpace']

/**
 * The context card's rows for `usage`: what the request carries largest
 * first, equal sizes in the frame's order, then Compaction buffer, then Free
 * space (`compaction.md` § What fills the context).
 */
export function contextCardRows(usage: ContextUsage): ContextCardRow[] {
  const rows = usage.rows.map((row) => ({ ...row, label: ROW_LABELS[row.kind] }))
  const carried = rows.filter((row) => !TRAILING_ROWS.includes(row.kind)).toSorted((a, b) => b.tokens - a.tokens)
  const trailing = TRAILING_ROWS.flatMap((kind) => rows.filter((row) => row.kind === kind))
  return [...carried, ...trailing]
}

/**
 * The room before compaction runs: the threshold, the window less the
 * compaction buffer, less the estimate, never below zero; null without a
 * window (`compaction.md` § What fills the context).
 */
export function untilCompaction(usage: ContextUsage): number | null {
  if (!usage.window) {
    return null
  }
  const buffer = usage.rows.find((row) => row.kind === 'compactionBuffer')?.tokens ?? 0
  return Math.max(0, usage.window - buffer - usage.tokens)
}

/**
 * Whether the context card lists its rows: closed at first, then as the user
 * last left it, kept in the browser for the next card.
 */
export function useContextListOpen(): Ref<boolean> {
  return useLocalStorage(CONTEXT_LIST_KEY, false)
}

/** Where the browser keeps whether the context card lists its rows. */
export const CONTEXT_LIST_KEY = 'demi-context-list-open'

const SHARE = new Intl.NumberFormat('en-US', {
  style: 'percent',
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
})

/** `tokens` as a share of `window` to a tenth of a percent, "24.2%"; a share too small to show reads "<0.1%". */
export function formatShare(tokens: number, window: number): string {
  const share = tokens / window
  return share > 0 && share < 0.0005 ? '<0.1%' : SHARE.format(share)
}

/**
 * The usage limits the context card shows under its rows: the label Settings
 * shows for the account the conversation's model runs on (empty when it has
 * none) and that account's windows
 * (`usage-and-quota.md` § What a snapshot says), and whether a visible card
 * asks the vendor for them again, as Settings does.
 */
export interface UsageLimits {
  accountLabel: string
  windows: SettingsQuotaWindow[]
  autoRefresh: boolean
  /** A manual refresh runs: the card's button turns and the windows stay as they were. */
  refreshing?: boolean
  /** The vendor's words for the last manual refresh that failed, shown under the header until the next answer. */
  error?: string | null
}

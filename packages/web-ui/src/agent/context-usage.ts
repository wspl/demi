import type { ContextPart, ContextRowKind, ContextUsage, InstructionEntry, PromptLayerKind } from '@demicodes/protocol'
import { baseName } from '../files/paths'
import type { SettingsQuotaWindow } from '../settings/types'
import { formatPercent } from '../ui/percent'
import { instructionRows } from './instructions'

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

const LAYER_LABELS: Record<PromptLayerKind, string> = {
  identity: 'Identity',
  harnessGuide: 'Harness guide',
  toolRules: 'Tool rules',
  modelIdentity: 'Model identity',
  profileInstructions: 'Instructions for this subagent',
}

/** What a context source is called where it is not a plugin's id. */
const SOURCE_LABELS: Record<string, string> = {
  execution: 'Execution context',
}

/** One part a row of the context card opens into. */
export interface ContextCardPart {
  key: string
  label: string
  /** The label is a command, a skill or a tool's name, set as code. */
  code: boolean
  tokens: number | null
  /** The instructions entry the part is, which a click opens. */
  entry: InstructionEntry | null
  /** The full path of a file, as the part's tip. */
  path: string | null
}

/** One row of the context card. */
export interface ContextCardRow {
  kind: ContextRowKind
  label: string
  tokens: number
  estimated: boolean
  parts: ContextCardPart[]
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
 * The context card's rows for `usage`, each with its parts. The Instructions
 * row lists `instructions`, what the newest instructions block holds, in its
 * order (`instructions.md` § What the card lists), each with the estimate the
 * session gave its file.
 */
export function contextCardRows(
  usage: ContextUsage,
  instructions: readonly InstructionEntry[],
): ContextCardRow[] {
  return usage.rows.map((row) => ({
    kind: row.kind,
    label: ROW_LABELS[row.kind],
    tokens: row.tokens,
    estimated: row.estimated,
    parts: row.kind === 'instructions' ? instructionParts(row.parts, instructions) : row.parts.map(cardPart),
  }))
}

function instructionParts(parts: readonly ContextPart[], entries: readonly InstructionEntry[]): ContextCardPart[] {
  return instructionRows(entries).map((row) => {
    const part = parts.find((part) => part.kind === 'instruction' && sameEntry(part.entry, row.entry))
    return {
      key: row.path ?? 'personal',
      label: row.label,
      code: false,
      tokens: part?.tokens ?? null,
      entry: row.entry,
      path: row.path,
    }
  })
}

function sameEntry(a: InstructionEntry, b: InstructionEntry): boolean {
  return a.kind === 'personal' ? b.kind === 'personal' : b.kind === 'file' && a.path === b.path
}

function cardPart(part: ContextPart, index: number): ContextCardPart {
  const plain = (label: string, code = false): ContextCardPart => ({
    key: `${part.kind}:${index}`,
    label,
    code,
    tokens: part.tokens,
    entry: null,
    path: null,
  })
  switch (part.kind) {
    case 'prompt':
      return plain(LAYER_LABELS[part.layer])
    case 'tool':
      // The definition of the one tool the model calls, apart from the calls of it under Messages.
      return plain(part.name === 'shell' ? 'Shell tool' : `${part.name} tool`)
    case 'commandGroup':
      return plain(part.group, true)
    case 'instruction':
      return { ...plain(part.entry.kind === 'personal' ? 'Personal instructions' : baseName(part.entry.path)), entry: part.entry }
    case 'skill':
      return plain(part.name, true)
    case 'source':
      return plain(SOURCE_LABELS[part.source] ?? part.source)
    case 'userMessages':
      return plain('Your messages')
    case 'agentText':
      return plain('Agent’s text')
    case 'thinking':
      return plain('Thinking')
    case 'images':
      return plain('Images and documents')
    case 'reports':
      return plain('Reports and agent messages')
    case 'unattributed':
      return plain('Unattributed')
  }
}

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
 * The usage limits the context card shows under its rows: the plan and the
 * windows of the account the conversation's model runs on
 * (`usage-and-quota.md` § What a snapshot says), and whether a visible card
 * asks the vendor for them again, as Settings does.
 */
export interface UsageLimits {
  plan: string
  windows: SettingsQuotaWindow[]
  autoRefresh: boolean
}

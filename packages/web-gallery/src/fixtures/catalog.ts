import type { ContextRow, ContextUsage } from '@demicodes/protocol'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'

export const demoProviders: ProviderInfo[] = [
  { id: 'anthropic', label: 'Anthropic', isAvailable: true },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
]

export const offlineProviders: ProviderInfo[] = demoProviders.map((provider) => ({
  ...provider,
  isAvailable: false,
}))

export const openaiOnlyProviders: ProviderInfo[] = [
  { id: 'anthropic', label: 'Anthropic', isAvailable: false },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
]

const fastTier = [{ id: 'priority', label: 'Fast', fast: true }]

function model(partial: Pick<ModelInfo, 'id' | 'name'> & Partial<ModelInfo>): ModelInfo {
  return {
    contextWindow: 200_000,
    contextLimit: null,
    acceptedExtensions: ['png', 'pdf', 'md'],
    reasoning: null,
    serviceTiers: null,
    ...partial,
  }
}

export const demoModels: Record<string, ModelInfo[]> = {
  anthropic: [
    model({
      id: 'claude-sonnet',
      name: 'Claude Sonnet',
      reasoning: {
        efforts: ['low', 'medium', 'high', 'max'],
        canDisable: true,
      },
      serviceTiers: fastTier,
    }),
    model({
      id: 'claude-opus',
      name: 'Claude Opus',
      // Over 500K: its menu offers the Context row, beside Fast Mode and Reasoning.
      contextWindow: 1_000_000,
      reasoning: {
        efforts: ['low', 'medium', 'high'],
        canDisable: false,
      },
      serviceTiers: fastTier,
    }),
  ],
  openai: [
    model({ id: 'gpt-5', name: 'GPT-5', serviceTiers: fastTier }),
  ],
}

/**
 * A compatible entry whose model was added by Custom, so its name is the id
 * the endpoint knows, as long as such ids run: the chip under a phone's
 * composer cuts it short and keeps the thinking level whole.
 */
export const longNameProviders: ProviderInfo[] = [
  { id: 'openrouter', label: 'OpenRouter', isAvailable: true },
]

export const longNameModels: Record<string, ModelInfo[]> = {
  openrouter: [
    model({
      id: 'deepseek/deepseek-v4.1-flash',
      name: 'deepseek/deepseek-v4.1-flash',
      reasoning: { efforts: ['low', 'medium', 'high'], canDisable: true },
    }),
  ],
}

/** The window the gallery's conversations use, as the backend reports it. */
const DEMO_WINDOW = 200_000

/** What a gallery conversation's context holds beside its messages. */
export interface UsageShape {
  /** The replayed summary of the last compaction, in tokens; none before the first. */
  summary?: number
}

/** The room the backend's threshold keeps in `window` for a model that writes 20,000 tokens or more. */
function compactionBuffer(window: number): number {
  return window - Math.max(window - 33_000, Math.floor(window / 2))
}

/**
 * The usage the backend reports at `tokens` of `window`, the window in use:
 * Compact is taken from half.
 *
 * Its rows are those a Demi conversation holds:
 *
 * - the system prompt with the shell tool;
 * - the commands;
 * - the instructions of `demoInstructions`;
 * - two skills;
 * - the execution context;
 * - Messages, the rest;
 * - in a window, the compaction buffer and the free space.
 *
 * The frame sends them in this order, and the card sorts them.
 */
export function usageOf(tokens: number, window: number | null, shape: UsageShape = {}): ContextUsage {
  const estimated = (kind: ContextRow['kind'], tokens: number): ContextRow => ({ kind, tokens, estimated: true })
  const fixed: ContextRow[] = [
    estimated('systemPrompt', 3_610),
    estimated('commands', 1_420),
    estimated('instructions', 3_240),
    estimated('skills', 410),
    estimated('otherContext', 480),
    ...(shape.summary ? [estimated('summary', shape.summary)] : []),
  ]
  const above = fixed.reduce((sum, row) => sum + row.tokens, 0)
  const messages = Math.max(0, tokens - above)
  const rows: ContextRow[] = [
    estimated('messages', messages),
    ...fixed,
  ]
  if (window !== null) {
    const buffer = compactionBuffer(window)
    rows.push(
      { kind: 'compactionBuffer', tokens: buffer, estimated: false },
      { kind: 'freeSpace', tokens: Math.max(0, window - Math.max(tokens, above) - buffer), estimated: false },
    )
  }
  return {
    tokens,
    window,
    compactFrom: window === null ? null : Math.floor(window / 2),
    rows: rows.filter((row) => row.tokens > 0),
    account: null,
  }
}

/** The backend's usage at `ratio` of the gallery conversations' window. */
export function usageAt(ratio: number, shape: UsageShape = {}): ContextUsage {
  return usageOf(Math.round(DEMO_WINDOW * ratio), DEMO_WINDOW, shape)
}

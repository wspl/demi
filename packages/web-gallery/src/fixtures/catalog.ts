import type { ContextPart, ContextRow, ContextUsage, PromptLayerKind } from '@demicodes/protocol'
import { demoInstructions } from './instructions'
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

/** The window the gallery's conversations use, as the backend reports it. */
const DEMO_WINDOW = 200_000

/** What a gallery conversation's context holds beside its messages. */
export interface UsageShape {
  /** The replayed summary of the last compaction, in tokens; none before the first. */
  summary?: number
}

/** The room the backend's threshold keeps in `window` for a model that writes 20,000 tokens or more (`compaction.md` § When compaction runs). */
function compactionBuffer(window: number): number {
  return window - Math.max(window - 33_000, Math.floor(window / 2))
}

/**
 * The usage the backend reports at `tokens` of `window`, the window in use:
 * Compact is taken from half. Its rows are those a Demi conversation holds:
 * the system prompt with the shell tool, the commands, the instructions of
 * `demoInstructions`, two skills and the execution context, then Messages,
 * the rest; in a window, the compaction buffer and the free space.
 */
export function usageOf(tokens: number, window: number | null, shape: UsageShape = {}): ContextUsage {
  const prompt = (layer: PromptLayerKind, tokens: number): ContextPart => ({ kind: 'prompt', layer, tokens })
  const group = (name: string, tokens: number): ContextPart => ({ kind: 'commandGroup', group: name, tokens })
  const fixed: ContextRow[] = [
    {
      kind: 'systemPrompt',
      tokens: 3_610,
      estimated: true,
      parts: [
        prompt('identity', 640),
        prompt('harnessGuide', 790),
        prompt('toolRules', 610),
        prompt('modelIdentity', 30),
        { kind: 'tool', name: 'shell', tokens: 1_540 },
      ],
    },
    {
      kind: 'commands',
      tokens: 1_420,
      estimated: true,
      parts: [
        group('demi browser', 340),
        group('demi agent', 210),
        group('demi file', 160),
        group('demi host', 150),
        group('demi shell', 130),
        group('demi attachment', 120),
        group('demi skills', 120),
      ],
    },
    {
      kind: 'instructions',
      tokens: 3_240,
      estimated: true,
      parts: [
        { kind: 'instruction', entry: demoInstructions[0]!, tokens: 40 },
        { kind: 'instruction', entry: demoInstructions[1]!, tokens: 2_610 },
        { kind: 'instruction', entry: demoInstructions[2]!, tokens: 540 },
      ],
    },
    {
      kind: 'skills',
      tokens: 410,
      estimated: true,
      parts: [
        { kind: 'skill', name: 'web-design-guidelines', tokens: 190 },
        { kind: 'skill', name: 'release', tokens: 150 },
      ],
    },
    {
      kind: 'otherContext',
      tokens: 480,
      estimated: true,
      parts: [{ kind: 'source', source: 'execution', tokens: 480 }],
    },
    ...(shape.summary ? [{ kind: 'summary' as const, tokens: shape.summary, estimated: true, parts: [] }] : []),
  ]
  const above = fixed.reduce((sum, row) => sum + row.tokens, 0)
  const messages = Math.max(0, tokens - above)
  // The messages' parts are block estimates; what they miss of the provider's count is Unattributed.
  const share = (part: number) => Math.round(messages * part)
  const parts: ContextPart[] = [
    { kind: 'commandGroup', group: 'demi browser', tokens: share(0.34) },
    { kind: 'commandGroup', group: 'shell', tokens: share(0.22) },
    { kind: 'agentText', tokens: share(0.12) },
    { kind: 'images', tokens: share(0.1) },
    { kind: 'thinking', tokens: share(0.07) },
    { kind: 'userMessages', tokens: share(0.05) },
    { kind: 'commandGroup', group: 'demi file', tokens: share(0.04) },
    { kind: 'reports', tokens: share(0.02) },
  ]
  const attributed = parts.reduce((sum, part) => sum + part.tokens, 0)
  parts.push({ kind: 'unattributed', tokens: messages - attributed })
  const rows: ContextRow[] = [
    { kind: 'messages', tokens: messages, estimated: true, parts: parts.filter((part) => part.tokens > 0) },
    ...fixed,
  ]
  if (window !== null) {
    const buffer = compactionBuffer(window)
    rows.push(
      { kind: 'compactionBuffer', tokens: buffer, estimated: false, parts: [] },
      { kind: 'freeSpace', tokens: Math.max(0, window - Math.max(tokens, above) - buffer), estimated: false, parts: [] },
    )
  }
  return {
    tokens,
    window,
    compactFrom: window === null ? null : Math.floor(window / 2),
    rows: rows.filter((row) => row.tokens > 0),
  }
}

/** The backend's usage at `ratio` of the gallery conversations' window. */
export function usageAt(ratio: number, shape: UsageShape = {}): ContextUsage {
  return usageOf(Math.round(DEMO_WINDOW * ratio), DEMO_WINDOW, shape)
}

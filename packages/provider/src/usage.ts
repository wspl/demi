/**
 * Token accounting shared by the OpenAI-shaped wire formats (`responses.ts`,
 * `chat-completions.ts`) and by any provider whose vendor reports counts the
 * same way.
 */
import type { TokenUsage } from '@demicodes/core'

/**
 * A `TokenUsage` from the counts an OpenAI-shaped API reports, where the input
 * count *includes* the cached prefix. Demi keeps the two apart, so the cached
 * tokens are subtracted from the input tokens rather than counted twice.
 * Absent counts are zero, which makes an absent usage object a zeroed usage.
 */
export function tokenUsageWithCachedInput(counts: {
  inputTokens?: number | null
  outputTokens?: number | null
  cachedTokens?: number | null
}): TokenUsage {
  const cacheReadTokens = counts.cachedTokens ?? 0
  return {
    inputTokens: Math.max(0, (counts.inputTokens ?? 0) - cacheReadTokens),
    outputTokens: counts.outputTokens ?? 0,
    cacheReadTokens,
    cacheWriteTokens: 0,
  }
}

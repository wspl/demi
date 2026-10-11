/**
 * The vendors' marks, monochrome SVGs (models.dev's, and Claude's own) that
 * `VendorMark` paints in the text's color: one for each models.dev vendor Demi
 * has a mark for, and each subscription wears its vendor's. A vendor without a
 * mark here shows its initial.
 */
import alibaba from './logos/alibaba.svg?url'
import amazonBedrock from './logos/amazon-bedrock.svg?url'
import anthropic from './logos/anthropic.svg?url'
import azure from './logos/azure.svg?url'
import cerebras from './logos/cerebras.svg?url'
import claude from './logos/claude.svg?url'
import deepseek from './logos/deepseek.svg?url'
import fireworksAi from './logos/fireworks-ai.svg?url'
import google from './logos/google.svg?url'
import googleVertex from './logos/google-vertex.svg?url'
import groq from './logos/groq.svg?url'
import minimax from './logos/minimax.svg?url'
import mistral from './logos/mistral.svg?url'
import moonshotai from './logos/moonshotai.svg?url'
import openai from './logos/openai.svg?url'
import openrouter from './logos/openrouter.svg?url'
import perplexity from './logos/perplexity.svg?url'
import togetherai from './logos/togetherai.svg?url'
import xai from './logos/xai.svg?url'
import zhipuai from './logos/zhipuai.svg?url'

const VENDOR_LOGOS: Record<string, string> = {
  alibaba,
  'amazon-bedrock': amazonBedrock,
  anthropic,
  azure,
  cerebras,
  deepseek,
  'fireworks-ai': fireworksAi,
  google,
  'google-vertex': googleVertex,
  groq,
  minimax,
  mistral,
  moonshotai,
  openai,
  openrouter,
  perplexity,
  togetherai,
  xai,
  zhipuai,
}

/** The vendor whose mark each subscription wears; Claude Code wears Claude's own. */
const SUBSCRIPTION_LOGOS: Record<string, string> = {
  'claude-code': claude,
  codex: openai,
  'grok-build': xai,
}

/** The mark of models.dev vendor `vendorId`; null when Demi has none for it. */
export function vendorLogo(vendorId: string | null): string | null {
  return (vendorId && VENDOR_LOGOS[vendorId]) || null
}

/**
 * The mark of a provider entry: a subscription's by its provider type, an
 * API key's by its vendor; null for a bare endpoint or a vendor Demi has no
 * mark for.
 */
export function providerLogo(entry: {
  kind: 'api_key' | 'subscription'
  providerType: string
  vendorId: string | null
}): string | null {
  return entry.kind === 'subscription'
    ? SUBSCRIPTION_LOGOS[entry.providerType] ?? null
    : vendorLogo(entry.vendorId)
}

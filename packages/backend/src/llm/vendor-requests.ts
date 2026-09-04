import type { OpenAIApiRequestOptions } from '@demicodes/provider-openai-api'

/** Vendor protocol requirements applied to every model in that vendor's catalog. */
const REQUESTS_BY_VENDOR: Record<string, OpenAIApiRequestOptions> = {
  deepseek: { passBackReasoningContent: true },
}

export function vendorRequestOptions(vendorId: string | undefined): OpenAIApiRequestOptions | undefined {
  return vendorId ? REQUESTS_BY_VENDOR[vendorId] : undefined
}

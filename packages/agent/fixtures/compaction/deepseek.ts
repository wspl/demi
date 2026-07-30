/**
 * Shared DeepSeek OpenAI-compatible provider wiring for the compaction fixture harness.
 * Auth: `DEEPSEEK_API_KEY` (optional `DEEPSEEK_BASE_URL`). Bun loads `.env` from the repo root.
 */
import type { ModelSelection } from '@demicodes/core'
import { modelSelectionFromCatalog, providerRuntime, type AgentProvider, type Provider } from '@demicodes/provider'
import {
  createOpenAIApiProvider,
  type OpenAIApiModelOptions,
  type OpenAIApiProviderOptions,
} from '@demicodes/provider-openai-api'

export const DEFAULT_DEEPSEEK_BASE_URL = 'https://api.deepseek.com/v1'
export const DEFAULT_FLASH_MODEL_ID = 'deepseek-v4-flash'

export interface DeepSeekFlashRuntime {
  provider: Provider
  runtime: AgentProvider
  model: ModelSelection
}

export function requireDeepSeekApiKey(): string {
  const key = process.env.DEEPSEEK_API_KEY?.trim()
  if (!key) {
    throw new Error('DEEPSEEK_API_KEY is not set (put it in the repo-root .env)')
  }
  return key
}

export function flashModelOptions(contextWindow: number, modelId = DEFAULT_FLASH_MODEL_ID): OpenAIApiModelOptions {
  return {
    id: modelId,
    displayName: 'DeepSeek V4 Flash',
    contextWindow,
    supportsTools: true,
    supportsAttachments: false,
    supportsReasoning: true,
    supportedThinkingEfforts: ['low', 'medium', 'high'],
    defaultThinkingEffort: null,
  }
}

export async function createDeepSeekFlash(contextWindow: number): Promise<DeepSeekFlashRuntime> {
  const apiKey = requireDeepSeekApiKey()
  const baseUrl = process.env.DEEPSEEK_BASE_URL?.trim() || DEFAULT_DEEPSEEK_BASE_URL
  const modelId = process.env.DEEPSEEK_FLASH_MODEL?.trim() || DEFAULT_FLASH_MODEL_ID
  const options = flashModelOptions(contextWindow, modelId)
  const provider = createOpenAIApiProvider({
    id: 'deepseek',
    displayName: 'DeepSeek',
    wireApi: 'chat-completions',
    baseUrl,
    apiKey: () => apiKey,
    models: [options],
    defaultModelId: modelId,
    request: { extraBody: { thinking: { type: 'disabled' } } },
    fetch: createDeepSeekUsageFetch(),
  })
  const catalog = await provider.listModels?.()
  const entry = catalog?.models.find((model) => model.id === modelId) ?? null
  const model = modelSelectionFromCatalog('deepseek', entry, { modelId })
  // Catalog metadata may advertise the vendor's full window; pin the registered window
  // so compaction thresholds stay under our control for the harness.
  model.model.contextWindow = contextWindow
  const runtime = await providerRuntime(provider, { providerId: 'deepseek', model })
  return { provider, runtime, model }
}

type FetchLike = NonNullable<OpenAIApiProviderOptions['fetch']>

/** Map DeepSeek `prompt_cache_hit_tokens` onto OpenAI-style `prompt_tokens_details.cached_tokens`. */
export function createDeepSeekUsageFetch(fetchImpl: FetchLike = fetch): FetchLike {
  return async (input, init) => {
    const response = await fetchImpl(input, init)
    const contentType = response.headers.get('content-type') ?? ''
    if (!response.ok || !response.body || !contentType.includes('text/event-stream')) return response

    const reader = response.body.getReader()
    const decoder = new TextDecoder()
    const encoder = new TextEncoder()
    let buffered = ''

    const body = new ReadableStream<Uint8Array>({
      async pull(controller) {
        const { value, done } = await reader.read()
        if (done) {
          if (buffered) controller.enqueue(encoder.encode(rewriteDeepSeekUsageLine(buffered)))
          controller.close()
          return
        }
        buffered += decoder.decode(value, { stream: true })
        let newline = buffered.indexOf('\n')
        while (newline !== -1) {
          const line = buffered.slice(0, newline + 1)
          buffered = buffered.slice(newline + 1)
          controller.enqueue(encoder.encode(rewriteDeepSeekUsageLine(line)))
          newline = buffered.indexOf('\n')
        }
      },
      cancel() {
        void reader.cancel()
      },
    })

    const headers = new Headers(response.headers)
    headers.delete('content-length')
    headers.delete('content-encoding')
    return new Response(body, { status: response.status, statusText: response.statusText, headers })
  }
}

function rewriteDeepSeekUsageLine(line: string): string {
  if (!line.includes('prompt_cache_hit_tokens')) return line
  return line.replace(/"prompt_cache_hit_tokens"\s*:\s*(\d+)/, '"prompt_tokens_details":{"cached_tokens":$1}')
}

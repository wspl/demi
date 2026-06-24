import type { Provider } from '@demi/provider'
import { createAnthropicApiProvider, type AnthropicApiModelOptions } from '@demi/provider-anthropic-api'
import { createClaudeCodeProvider } from '@demi/provider-claude-code'
import { createCodexProvider } from '@demi/provider-codex'
import { createOpenAIApiProvider, type OpenAIApiModelOptions } from '@demi/provider-openai-api'
import type { ProviderId, ServerOptions } from './server-options'

export function createWebProviders(options: ServerOptions): Provider[] {
  const claudeCodeProvider = createClaudeCodeProvider({ claudePath: options.claudePath })
  const codexProvider = createCodexProvider({
    codexHome: options.codexHome,
    baseUrl: options.provider === 'codex' ? options.baseUrl : undefined,
    transport: options.transport,
  })
  const openAIModels = apiModelOptionsFor(options, 'openai')
  const openAIProvider = createOpenAIApiProvider({
    baseUrl: options.provider === 'openai' ? options.baseUrl : undefined,
    wireApi: options.openAIWireApi,
    models: openAIModels,
    defaultModelId: openAIModels?.[0]?.id,
  })
  const anthropicModels = apiModelOptionsFor(options, 'anthropic')
  const anthropicProvider = createAnthropicApiProvider({
    baseUrl: options.provider === 'anthropic' ? options.baseUrl : undefined,
    models: anthropicModels,
    defaultModelId: anthropicModels?.[0]?.id,
  })

  return orderProviders([claudeCodeProvider, codexProvider, openAIProvider, anthropicProvider], options.provider)
}

function apiModelOptionsFor(options: ServerOptions, providerId: Extract<ProviderId, 'openai'>): OpenAIApiModelOptions[] | undefined
function apiModelOptionsFor(options: ServerOptions, providerId: Extract<ProviderId, 'anthropic'>): AnthropicApiModelOptions[] | undefined
function apiModelOptionsFor(
  options: ServerOptions,
  providerId: Extract<ProviderId, 'openai' | 'anthropic'>,
): Array<OpenAIApiModelOptions | AnthropicApiModelOptions> | undefined {
  if (options.provider !== providerId || !options.modelId) return undefined
  const supportedThinkingEfforts = options.modelThinkingEfforts ? [...options.modelThinkingEfforts] : undefined
  const defaultThinkingEffort = defaultThinkingEffortFor(options)
  return [
    {
      id: options.modelId,
      displayName: options.modelDisplayName ?? undefined,
      contextWindow: options.modelContextWindow!,
      supportsReasoning: supportedThinkingEfforts ? true : undefined,
      supportedThinkingEfforts,
      defaultThinkingEffort: defaultThinkingEffort ?? undefined,
      canDisableThinking: options.modelCanDisableThinking ?? undefined,
    },
  ]
}

function defaultThinkingEffortFor(options: ServerOptions): string | null {
  return options.thinkingEffort ?? (options.modelCanDisableThinking === false ? options.modelThinkingEfforts?.[0] ?? null : null)
}

function orderProviders(providers: Provider[], selectedProviderId: string): Provider[] {
  const selected = providers.find((provider) => provider.id === selectedProviderId)
  return selected ? [selected, ...providers.filter((provider) => provider.id !== selectedProviderId)] : providers
}

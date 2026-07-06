import process from 'node:process'
import { errorMessage } from '@demicodes/utils'
import type { ModelSelection, ThinkingEffort } from '@demicodes/core'
import { modelSelectionFromCatalog } from '@demicodes/provider'
import type { Provider, ProviderModel, ProviderModelList } from '@demicodes/provider'
import { createAnthropicApiProvider } from '@demicodes/provider-anthropic-api'
import { createClaudeCodeProvider } from '@demicodes/provider-claude-code'
import { codexAuthStatus, createCodexProvider } from '@demicodes/provider-codex'
import { createOpenAIApiProvider } from '@demicodes/provider-openai-api'
import { writeEventLine } from './output'
import type { ReplOptions } from './options'

export interface ResolvedReplModel {
  selection: ModelSelection
  warnings: string[]
  catalog: ProviderModelList | null
}

export async function resolveReplModel(
  provider: Provider,
  options: ReplOptions,
): Promise<ResolvedReplModel> {
  if (options.modelId) {
    validateExplicitModelId(options.provider, options.modelId)
    if (options.serviceTierId) throw new Error('--service-tier requires catalog-backed model selection')
    return {
      selection: modelSelectionFromCatalogModel(options.provider, options.modelId, options.thinkingEffort, options.serviceTierId, null),
      warnings: [],
      catalog: null,
    }
  }

  let catalog: ProviderModelList | null = null
  let catalogError: unknown = null
  try {
    if (!provider.listModels) throw new Error(`Provider ${options.provider} does not expose a model catalog`)
    catalog = await provider.listModels()
  } catch (error) {
    catalogError = error
  }

  if (!catalog) {
    throw new Error(`Unable to load ${options.provider} model catalog: ${errorMessage(catalogError)}`)
  }
  if (catalog.models.length === 0) {
    throw new Error(`${options.provider} model catalog returned no models`)
  }
  const selected =
    (catalog.defaultModelId ? catalog.models.find((model) => model.id === catalog.defaultModelId) : null) ?? catalog.models[0]
  if (!selected) throw new Error(`${options.provider} model catalog returned no selectable models`)
  validateThinkingEffortForCatalogModel(options.thinkingEffort, selected)
  validateServiceTierForCatalogModel(options.serviceTierId, selected)
  return {
    selection: modelSelectionFromCatalogModel(options.provider, selected.id, options.thinkingEffort, options.serviceTierId, selected),
    warnings: [...catalog.warnings],
    catalog,
  }
}

function validateExplicitModelId(provider: ReplOptions['provider'], modelId: string): void {
  if (modelId === 'opus' || modelId === 'sonnet' || modelId === 'haiku' || modelId === 'default') {
    throw new Error(`--model must be a full ${provider} model id, not alias "${modelId}"`)
  }
  if (provider === 'claude-code' && !modelId.startsWith('claude-')) {
    throw new Error('--model for claude-code must be a full Claude model id such as claude-opus-4-8')
  }
  if (provider === 'codex' && !modelId.startsWith('gpt-') && !modelId.startsWith('codex-')) {
    throw new Error('--model for codex must be a full Codex model id such as gpt-5.5')
  }
}

function validateThinkingEffortForCatalogModel(thinkingEffort: ThinkingEffort | null, model: ProviderModel): void {
  if (!thinkingEffort) return
  const supported = model.supportedThinkingEfforts
  if (!supported || supported.length === 0) {
    throw new Error(`Model ${model.id} does not advertise explicit thinking effort controls`)
  }
  if (!supported.includes(thinkingEffort)) {
    throw new Error(`Model ${model.id} does not support thinking effort "${thinkingEffort}"`)
  }
}

function validateServiceTierForCatalogModel(serviceTierId: string | null, model: ProviderModel): void {
  if (!serviceTierId) return
  const tiers = model.serviceTiers
  if (!tiers || tiers.length === 0) {
    throw new Error(`Model ${model.id} does not advertise service tier controls`)
  }
  if (!tiers.some((tier) => tier.id === serviceTierId)) {
    throw new Error(`Model ${model.id} does not support service tier "${serviceTierId}"`)
  }
}

function modelSelectionFromCatalogModel(
  provider: ReplOptions['provider'],
  modelId: string,
  thinkingEffort: ThinkingEffort | null,
  serviceTierId: string | null,
  model: ProviderModel | null,
): ModelSelection {
  return modelSelectionFromCatalog(provider, model, {
    modelId,
    thinking: thinkingEffort ? { type: 'effort', effort: thinkingEffort, summary: null } : null,
    serviceTierId,
    fallbackName: `${providerDisplayName(provider)} ${modelId}`,
  })
}

export function createReplProviders(options: ReplOptions): Provider[] {
  const claudeCodeProvider = createClaudeCodeProvider({ claudePath: options.claudePath })
  const codexProvider = createCodexProvider({
    codexHome: options.codexHome,
    baseUrl: options.provider === 'codex' ? options.baseUrl : undefined,
    transport: options.transport,
  })
  const openAIProvider = createOpenAIApiProvider({
    baseUrl: options.provider === 'openai' ? options.baseUrl : undefined,
    wireApi: options.openAIWireApi,
  })
  const anthropicProvider = createAnthropicApiProvider({
    baseUrl: options.provider === 'anthropic' ? options.baseUrl : undefined,
  })

  return orderProviders([claudeCodeProvider, codexProvider, openAIProvider, anthropicProvider], options.provider)
}

export function providerFor(providers: Provider[], id: string): Provider {
  const provider = providers.find((candidate) => candidate.id === id)
  if (!provider) throw new Error(`Provider ${id} is not configured`)
  return provider
}

export async function printCodexAuthStatus(options: ReplOptions): Promise<void> {
  const auth = await codexAuthStatus({ codexHome: options.codexHome })
  writeEventLine(
    process.stdout,
    'auth',
    `codex ${auth.status}${'accountLabel' in auth && auth.accountLabel ? ` (${auth.accountLabel})` : ''}${'message' in auth && auth.message ? ` (${auth.message})` : ''}`,
    auth.status === 'authenticated' ? 'green' : 'yellow',
  )
}

function orderProviders(providers: Provider[], selectedProviderId: string): Provider[] {
  const selected = providers.find((provider) => provider.id === selectedProviderId)
  return selected ? [selected, ...providers.filter((provider) => provider.id !== selectedProviderId)] : providers
}

export function providerDisplayName(provider: ReplOptions['provider']): string {
  if (provider === 'codex') return 'Codex'
  if (provider === 'openai') return 'OpenAI API'
  if (provider === 'anthropic') return 'Anthropic API'
  return 'Claude Code'
}

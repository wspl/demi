import type { ModelInfo } from '@demicodes/web-ui/transport/protocol'
import type {
  SettingsProviderEntry,
  SettingsProviderModel,
  SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import type {
  BackendProvider,
  CatalogModel,
  CatalogProvider,
} from '../api/contracts'
import type { LocalState } from './local'

export interface ProductProvider extends SettingsProviderEntry {
  providerType: string
  configured: boolean
  keyConfigured: boolean
}

const subscriptionNames: Record<string, string> = {
  'claude-code': 'Claude Code',
  codex: 'Codex',
  'grok-build': 'Grok Build',
}
export function subscriptionName(providerType: string): string {
  return subscriptionNames[providerType] ?? providerType
}

export function wireApi(
  providerType: string,
  wire: 'responses' | 'chat-completions' | null | undefined,
): SettingsWireApi {
  if (providerType === 'anthropic') {
    return 'anthropic-messages'
  }
  if (providerType === 'google') {
    return 'google-generative'
  }
  return wire === 'chat-completions' ? 'openai-chat' : 'openai-responses'
}

export function modelInfo(model: CatalogModel): ModelInfo {
  const metadata = model.selection.model
  return {
    id: model.id,
    name: model.displayName,
    contextWindow: model.contextWindow,
    inputLimit: metadata.inputLimit,
    acceptedExtensions: metadata.acceptedExtensions,
    reasoning: model.supportedThinkingEfforts?.length
      ? {
          efforts: model.supportedThinkingEfforts,
          defaultEffort: model.defaultThinkingEffort,
          canDisable: model.canDisableThinking !== false,
        }
      : null,
    serviceTiers:
      model.serviceTiers?.map((tier) => ({
        ...tier,
        fast: tier.fast === true,
      })) ?? null,
  }
}

export function settingsModel(
  model: CatalogModel,
  hidden: readonly string[],
): SettingsProviderModel {
  return {
    id: model.id,
    name: model.displayName,
    contextWindow: model.contextWindow,
    outputLimit: model.outputLimit,
    efforts: model.supportedThinkingEfforts ?? [],
    extensions:
      model.selection.model.acceptedExtensions?.map(
        (extension) => `.${extension}`,
      ) ?? null,
    fastTier: model.serviceTiers?.find((tier) => tier.fast)?.id ?? null,
    enabled: !hidden.includes(model.id),
  }
}

export function providerView(
  entry: BackendProvider,
  catalog: CatalogProvider | undefined,
  local: LocalState,
): ProductProvider {
  const details = entry.details
  const auth = details?.auth.status
  const runtime = details?.runtime.status
  const state =
    entry.error || auth === 'error' || runtime === 'error'
      ? 'error'
      : auth === 'unauthenticated'
        ? 'signed-out'
        : runtime === 'unavailable'
          ? 'unreachable'
          : details
            ? 'ready'
            : 'unconfigured'
  return {
    id: entry.id,
    name: entry.label,
    kind: entry.kind,
    providerType: entry.providerType,
    configured: true,
    keyConfigured: entry.keyConfigured,
    vendorId: entry.vendorId,
    baseUrl: entry.baseUrl ?? '',
    wireApi: wireApi(entry.providerType, entry.wireApi),
    apiKey: '',
    modelSource: entry.models ? 'manual' : 'catalog',
    catalogFetched: catalog?.sourceFetchedAt ?? null,
    stale: catalog?.stale ?? false,
    state,
    detail: entry.error ?? details?.auth.message ?? details?.runtime.message,
    enabled: !local.hiddenProviders.includes(entry.id),
    models:
      catalog?.models.map((model) =>
        settingsModel(model, local.hiddenModels[entry.id] ?? []),
      ) ?? [],
    accounts:
      details?.accounts.map((account) => ({
        id: account.id,
        label: account.label,
        plan: account.detail ?? '',
        active: account.id === details.active?.credentialId,
        quota:
          account.id === details.active?.credentialId
            ? (details.quota?.windows.flatMap((window) => {
                if (window.usedPercent === null) {
                  return []
                }
                return [
                  {
                    id: window.id,
                    label: window.label ?? window.id.replaceAll('_', ' '),
                    used: window.usedPercent,
                    max: 100,
                    resets: window.resetsAt,
                  },
                ]
              }) ?? [])
            : [],
      })) ?? [],
    logo: null,
  }
}

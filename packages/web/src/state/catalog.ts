import type { ModelInfo } from '@demicodes/web-ui/transport/protocol'
import type {
  SettingsProviderEntry,
  SettingsProviderModel,
  SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import { providerLogo } from '@demicodes/web-ui/settings/vendor-logos'
import { appliedContextLimit, type AuthState, type RuntimeState } from '@demicodes/protocol'
import { titleCase } from '@demicodes/utils'
import type {
  AccountDto,
  CatalogModel,
  CatalogProvider,
  ProviderState,
} from '../api/generated/web-api'
import type { LocalState } from './local'

/**
 * The package of the CLI that `providerId`'s requests start on the Cloud, by
 * the catalog; null when its requests are HTTP or the catalog lacks it.
 */
export function cliPackageOf(catalog: readonly CatalogProvider[], providerId: string): string | null {
  return catalog.find((provider) => provider.providerId === providerId)?.cliPackage ?? null
}

export interface ProductProvider extends SettingsProviderEntry {
  providerType: string
  /** The package of the CLI its requests start on the Cloud; null when they are HTTP. */
  cliPackage: string | null
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

/** A catalog model as the composer takes it, with `limit`, the context limit the user stored on it. */
export function modelInfo(model: CatalogModel, limit: number | null): ModelInfo {
  const metadata = model.selection.model
  return {
    id: model.id,
    name: model.displayName,
    contextWindow: model.contextWindow,
    contextLimit: appliedContextLimit(model.contextWindow, limit),
    acceptedExtensions: metadata.acceptedExtensions,
    reasoning: model.supportedThinkingEfforts?.length
      ? {
          efforts: model.supportedThinkingEfforts,
          canDisable: model.canDisableThinking !== false,
        }
      : null,
    serviceTiers: model.serviceTiers.map((tier) => ({ id: tier.id, label: tier.label, fast: tier.fast })),
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
    fastTier: model.serviceTiers.find((tier) => tier.fast)?.id ?? null,
    enabled: !hidden.includes(model.id),
  }
}

/** What a provider's health says in words, when it says anything. */
function healthMessage(health: AuthState | RuntimeState): string | undefined {
  return 'message' in health ? health.message : undefined
}

/**
 * The families whose account detail is the plan its sign-in recorded, such as
 * Claude Code's subscription type: their usage answer names no plan. Another
 * family's detail says how the account signed in (`chatgpt`, `oidc`), which
 * is not a plan.
 */
const PLAN_IN_DETAIL = new Set(['claude-code'])

/**
 * The plan an account is on (`usage-and-quota.md` § What a snapshot says):
 * what its quota snapshot reports, else the plan its sign-in recorded, in
 * title case ("max" reads "Max"); empty when neither says.
 */
export function accountPlan(
  providerType: string,
  account: Pick<AccountDto, 'detail' | 'quota'>,
): string {
  const reported = account.quota?.plan?.label || account.quota?.plan?.id
  if (reported) {
    return reported
  }
  return PLAN_IN_DETAIL.has(providerType) && account.detail ? titleCase(account.detail) : ''
}

export function providerView(
  entry: ProviderState,
  catalog: CatalogProvider | undefined,
  local: LocalState,
): ProductProvider {
  const details = entry.details.type === 'read' ? entry.details : null
  const auth = details?.auth.status
  const runtime = details?.runtime.status
  const state =
    !details || auth === 'error' || runtime === 'error'
      ? 'error'
      : auth === 'unauthenticated'
        ? 'signed-out'
        : runtime === 'unavailable'
          ? 'unreachable'
          : 'ready'
  return {
    id: entry.id,
    name: entry.label,
    kind: entry.kind,
    providerType: entry.providerType,
    configured: true,
    cliPackage: details?.cliPackage ?? catalog?.cliPackage ?? null,
    // An API-key entry is created with its key, which is kept until the entry goes.
    keyConfigured: entry.kind === 'api_key',
    vendorId: entry.vendorId,
    baseUrl: entry.baseUrl ?? '',
    wireApi: wireApi(entry.providerType, entry.wireApi),
    apiKey: '',
    modelSource: entry.models ? 'manual' : 'catalog',
    catalogFetched: catalog?.sourceFetchedAt ?? null,
    state,
    // A healthy provider's messages are notes ("Uses the … API"), not failures.
    detail: state === 'ready'
      ? undefined
      : entry.details.type === 'failed'
        ? entry.details.message
        : healthMessage(entry.details.auth) ?? healthMessage(entry.details.runtime),
    enabled: !local.hiddenProviders.includes(entry.id),
    models:
      catalog?.models.map((model) =>
        settingsModel(model, local.hiddenModels[entry.id] ?? []),
      ) ?? [],
    accounts:
      details?.accounts.map((account) => ({
        id: account.id,
        label: account.label,
        plan: accountPlan(entry.providerType, account),
        active: account.id === details.active,
        quota: account.quota?.windows.flatMap((window) => {
          if (window.usedPercent === null) {
            return []
          }
          return [
            {
              id: window.id,
              label: window.label,
              used: window.usedPercent,
              max: 100,
              resetsAt: window.resetsAt,
            },
          ]
        }) ?? [],
      })) ?? [],
    autoRefreshUsage: details?.quotaCapability.type === 'supported'
      && details.quotaCapability.probe === 'free',
    logo: providerLogo(entry),
  }
}

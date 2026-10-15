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

/** A catalog model as Settings › Providers lists it; `enabled` is whether the user has it on. */
export function settingsModel(
  model: CatalogModel,
  enabled: boolean,
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
    enabled,
  }
}

/** Which entries and models the user turned off, as the preferences say. */
export interface Hidden {
  provider(providerId: string): boolean
  model(providerId: string, modelId: string): boolean
}

/**
 * Why an incomplete entry cannot infer yet, from its catalog's availability
 * (`models.md` § A provider is in the list from the start); null once it has
 * everything it needs to try.
 */
export function setupOf(catalog: CatalogProvider | undefined): { message: string } | null {
  const availability = catalog?.availability
  return availability?.type === 'unavailable' && availability.reason === 'setup'
    ? { message: availability.message }
    : null
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
  hidden: Hidden,
): ProductProvider {
  const details = entry.details.type === 'read' ? entry.details : null
  const auth = details?.auth.status
  const runtime = details?.runtime.status
  const setup = setupOf(catalog)
  const state =
    setup
      ? 'setup'
      : !details || auth === 'error' || runtime === 'error'
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
    keyConfigured: entry.hasApiKey,
    vendorId: entry.vendorId,
    compatible: entry.compatible,
    baseUrl: entry.endpoint ?? '',
    wireApi: wireApi(entry.providerType, entry.wireApi),
    apiKey: '',
    // A compatible API has no catalog: its list is the one the user keeps.
    modelSource: entry.models || entry.compatible ? 'manual' : 'catalog',
    // The Unix epoch is what a list that was never downloaded reports: no time to show.
    catalogFetched: catalog && Date.parse(catalog.sourceFetchedAt) > 0 ? catalog.sourceFetchedAt : null,
    state,
    // A healthy provider's messages are notes ("Uses the … API"), not failures;
    // an incomplete one says what it lacks on its fields.
    detail: state === 'ready' || state === 'setup'
      ? undefined
      : entry.details.type === 'failed'
        ? entry.details.message
        : healthMessage(entry.details.auth) ?? healthMessage(entry.details.runtime),
    enabled: !hidden.provider(entry.id),
    models:
      catalog?.models.map((model) =>
        settingsModel(model, !hidden.model(entry.id, model.id)),
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
              name: { kind: window.kind, periodSeconds: window.periodSeconds ?? null, scope: window.scope },
              used: window.usedPercent,
              max: 100,
              resetsAt: window.resetsAt,
            },
          ]
        }) ?? [],
      })) ?? [],
    rotation: entry.kind === 'subscription' ? entry.rotation : undefined,
    autoRefreshUsage: details?.quotaCapability.type === 'supported'
      && details.quotaCapability.probe === 'free',
    logo: providerLogo(entry),
  }
}

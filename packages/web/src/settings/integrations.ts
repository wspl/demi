import { t } from './locales/text'
import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { IntegrationChange, ServerSizes, SettingsIntegration } from '@demicodes/web-ui/settings/integrations'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import {
  integrationAnswerSchema,
  integrationWithTokenSchema,
  serverSizesSchema,
  type IntegrationDto,
  type IntegrationPatch,
} from '../api/generated/web-api'
import { useHeldList, useProduct, type Answer } from '../state/product'

/** A refusal the user fixes in the field, such as an empty name. */
function refusedField(error: unknown): boolean {
  return error instanceof ApiError && error.status >= 400 && error.status < 500 && error.status !== 404
}

/** An integration as the page shows it: an answer that leaves out a value has none. */
function shown(integration: IntegrationDto): SettingsIntegration {
  const sizing = integration.cloudSizing
  return {
    ...integration,
    initScript: integration.initScript ?? null,
    cloudSizing: {
      cpus: sizing.cpus ?? null,
      memoryMib: sizing.memoryMib ?? null,
      systemGb: sizing.systemGb ?? null,
      homeGb: sizing.homeGb ?? null,
      sharedGb: sizing.sharedGb ?? null,
    },
    cloudIdleMinutes: integration.cloudIdleMinutes ?? null,
  }
}

/** The answer that leaves the integrations as `next` makes them of those the page last read. */
function integrationsAnswer(next: (current: IntegrationDto[]) => IntegrationDto[]): Answer {
  return (read) => ({ type: 'integrations', integrations: next(read.integrations ?? []) })
}

/** The integrations, oldest first, with `integration` in its place by id. */
function withIntegration(integrations: IntegrationDto[], integration: IntegrationDto): IntegrationDto[] {
  return integrations.some((candidate) => candidate.id === integration.id)
    ? integrations.map((candidate) => (candidate.id === integration.id ? integration : candidate))
    : [...integrations, integration]
}

/**
 * The user's integrations as Settings › Integrations reads and changes them
 * (`crates/backend-user-shard/src/integrations.rs`): a change shows its
 * answer, and the synchronization channel's `integrations` part brings
 * every change, an agent's `demi integrations` among them, to every page. A
 * token is held only from the answer that brought it until the page closes,
 * and shown only while the integration's fingerprint is still the token's:
 * a token another page or an agent replaced is gone from the page.
 */
const useIntegrationStore = defineStore('integration-settings', () => {
  const product = useProduct()
  const integrations = computed<SettingsIntegration[]>(() => (product.snapshot?.integrations ?? []).map(shown))
  /** Each token the page was answered, with the fingerprint of the integration as the answer brought it. */
  const held = ref<Record<string, { token: string; fingerprint: string }>>({})
  const tokens = computed<Record<string, string>>(() => {
    const current: Record<string, string> = {}
    for (const integration of product.snapshot?.integrations ?? []) {
      const kept = held.value[integration.id]
      if (kept?.fingerprint === integration.tokenFingerprint)
        current[integration.id] = kept.token
    }
    return current
  })

  function hold(integration: IntegrationDto, token: string): void {
    held.value = { ...held.value, [integration.id]: { token, fingerprint: integration.tokenFingerprint } }
  }

  function put(answer: IntegrationDto, at: number): void {
    product.answered(at, integrationsAnswer((current) => withIntegration(current, answer)))
  }

  /** Add Integration: one at once, with its token; answers its id, or null when it failed. */
  async function add(): Promise<string | null> {
    try {
      const at = product.sent()
      const response = await apiRequest('/integrations', { method: 'POST' })
      const { integration, token } = await readResponse(response, integrationWithTokenSchema)
      put(integration, at)
      hold(integration, token)
      return integration.id
    } catch (error) {
      reportError(t('integrations.addFailed'), error, { site: 'integrations.addFailed', userVisible: true })
      return null
    }
  }

  /** Writes a change; a refusal the user can fix is thrown for its field, any other failure is a toast. */
  async function change(id: string, change: IntegrationChange): Promise<void> {
    try {
      const at = product.sent()
      const response = await apiRequest(`/integrations/${encodeURIComponent(id)}`, {
        method: 'PATCH',
        ...jsonBody(change satisfies IntegrationPatch),
      })
      put((await readResponse(response, integrationAnswerSchema)).integration, at)
    } catch (error) {
      if (refusedField(error))
        throw error
      reportError(t('integrations.saveFailed'), error, { site: 'integrations.saveFailed', userVisible: true })
    }
  }

  /** Replace Token: the new token shows on the page in the old one's place. */
  async function replaceToken(id: string): Promise<void> {
    try {
      const at = product.sent()
      const response = await apiRequest(`/integrations/${encodeURIComponent(id)}/token`, { method: 'POST' })
      const { integration, token } = await readResponse(response, integrationWithTokenSchema)
      put(integration, at)
      hold(integration, token)
    } catch (error) {
      reportError(t('integrations.replaceFailed'), error, { site: 'integrations.replaceFailed', userVisible: true })
    }
  }

  /** Delete Integration: answers whether it went, so its page closes. */
  async function remove(id: string): Promise<boolean> {
    try {
      const at = product.sent()
      await apiRequest(`/integrations/${encodeURIComponent(id)}`, { method: 'DELETE' })
      product.answered(at, integrationsAnswer((current) => current.filter((integration) => integration.id !== id)))
      const { [id]: _gone, ...kept } = held.value
      held.value = kept
      return true
    } catch (error) {
      reportError(t('integrations.deleteFailed'), error, { site: 'integrations.deleteFailed', userVisible: true })
      return false
    }
  }

  /** The page closed: the tokens it showed go. */
  function forgetTokens(): void {
    held.value = {}
  }

  const serverSizes = ref<ServerSizes | null>(null)

  /** Reads what a Cloud gets for each size an integration leaves to the server. */
  async function readServerSizes(): Promise<void> {
    try {
      const sizes = await readResponse(await apiRequest('/integrations/server-sizes'), serverSizesSchema)
      serverSizes.value = {
        ...sizes,
        cpus: sizes.cpus ?? null,
        memoryMib: sizes.memoryMib ?? null,
        serverCpus: sizes.serverCpus ?? null,
        serverMemoryMib: sizes.serverMemoryMib ?? null,
      }
    } catch (error) {
      reportError(t('integrations.sizesFailed'), error, { site: 'integrations.sizesFailed' })
    }
  }

  return { integrations, tokens, serverSizes, add, change, replaceToken, remove, forgetTokens, readServerSizes }
})

/** The integration settings, which hold the user's integrations while a view shows them. */
export function useIntegrationSettings(): ReturnType<typeof useIntegrationStore> {
  useHeldList('integrations')
  return useIntegrationStore()
}

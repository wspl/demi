import { t } from './locales/text'
import { ref } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { IntegrationChange, SettingsIntegration } from '@demicodes/web-ui/settings/integrations'
import { apiRequest, ApiError, jsonBody, readResponse } from '../api/client'
import {
  integrationAnswerSchema,
  integrationListSchema,
  integrationWithTokenSchema,
  type IntegrationDto,
  type IntegrationPatch,
} from '../api/generated/web-api'

/** A refusal the user fixes in the field, such as an empty name. */
function refusedField(error: unknown): boolean {
  return error instanceof ApiError && error.status >= 400 && error.status < 500 && error.status !== 404
}

/**
 * The user's integrations as Settings › Integrations reads and changes them
 * (`crates/backend-user-shard/src/integrations.rs`). The list is read when the
 * page opens; each change puts its answer in its place. A token is held only
 * from the answer that brought it until the page closes.
 */
export const useIntegrationSettings = defineStore('integration-settings', () => {
  const integrations = ref<SettingsIntegration[]>([])
  const load = ref<'loading' | 'ready' | 'failed'>('loading')
  const tokens = ref<Record<string, string>>({})

  function put(integration: IntegrationDto): void {
    const list = integrations.value
    integrations.value = list.some((candidate) => candidate.id === integration.id)
      ? list.map((candidate) => (candidate.id === integration.id ? integration : candidate))
      : [...list, integration]
  }

  async function read(): Promise<void> {
    load.value = 'loading'
    try {
      const response = await apiRequest('/integrations')
      integrations.value = (await readResponse(response, integrationListSchema)).integrations
      load.value = 'ready'
    } catch (error) {
      load.value = 'failed'
      reportError(t('integrations.loadFailed'), error, { site: 'integrations.loadFailed' })
    }
  }

  /** Add Integration: one at once, with its token; answers its id, or null when it failed. */
  async function add(): Promise<string | null> {
    try {
      const response = await apiRequest('/integrations', { method: 'POST' })
      const { integration, token } = await readResponse(response, integrationWithTokenSchema)
      put(integration)
      tokens.value = { ...tokens.value, [integration.id]: token }
      return integration.id
    } catch (error) {
      reportError(t('integrations.addFailed'), error, { site: 'integrations.addFailed', userVisible: true })
      return null
    }
  }

  /** Writes a change; a refusal the user can fix is thrown for its field, any other failure is a toast. */
  async function change(id: string, change: IntegrationChange): Promise<void> {
    try {
      const response = await apiRequest(`/integrations/${encodeURIComponent(id)}`, {
        method: 'PATCH',
        ...jsonBody(change satisfies IntegrationPatch),
      })
      put((await readResponse(response, integrationAnswerSchema)).integration)
    } catch (error) {
      if (refusedField(error))
        throw error
      reportError(t('integrations.saveFailed'), error, { site: 'integrations.saveFailed', userVisible: true })
    }
  }

  /** Replace Token: the new token shows on the page in the old one's place. */
  async function replaceToken(id: string): Promise<void> {
    try {
      const response = await apiRequest(`/integrations/${encodeURIComponent(id)}/token`, { method: 'POST' })
      const { integration, token } = await readResponse(response, integrationWithTokenSchema)
      put(integration)
      tokens.value = { ...tokens.value, [integration.id]: token }
    } catch (error) {
      reportError(t('integrations.replaceFailed'), error, { site: 'integrations.replaceFailed', userVisible: true })
    }
  }

  /** The page closed: the tokens it showed go. */
  function forgetTokens(): void {
    tokens.value = {}
  }

  return { integrations, load, tokens, read, add, change, replaceToken, forgetTokens }
})

import { t } from './locales/text'
import { computed } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { SettingsCredential, SettingsCredentialDraft } from '@demicodes/web-ui/settings/types'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import {
  credentialAnswerSchema,
  credentialValuesSchema,
  type CredentialDto,
  type CredentialPatch,
  type NewCredential,
} from '../api/generated/web-api'
import { useProduct, type Answer } from '../state/product'

/** The credentials in name order, as the backend lists them: `credential` in its place by id. */
function withCredential(credentials: CredentialDto[], credential: CredentialDto): CredentialDto[] {
  const others = credentials.filter((candidate) => candidate.id !== credential.id)
  return [...others, credential].sort((a, b) => a.name.localeCompare(b.name))
}

/** The answer that leaves the credentials as `next` makes them of those the page last read. */
function credentialsAnswer(next: (current: CredentialDto[]) => CredentialDto[]): Answer {
  return (read) => ({ type: 'credentials', credentials: next(read.credentials) })
}

/**
 * The user's credentials as Settings › Credentials changes them
 * (`web-api-conversations.md` § Credentials): a saved or deleted credential shows its
 * answer, and the synchronization channel's `credentials` part brings the
 * same to every other page, each visible field with its value and never a
 * secret one's. A secret value comes only from the reveal, which the page
 * shows and does not keep.
 */
export const useCredentialSettings = defineStore('credential-settings', () => {
  const product = useProduct()
  const credentials = computed<SettingsCredential[]>(() =>
    (product.snapshot?.credentials ?? []).map((credential) => ({
      id: credential.id,
      name: credential.name,
      note: credential.note,
      fields: credential.fields.map(({ value, ...field }) => ({ ...field, value: value ?? null })),
      keepUntil: credential.keepUntil,
      lastUsedAt: credential.lastUsedAt,
    })),
  )

  /** Creates a credential, or changes the credential `id`; a refusal is thrown for the editor to show. */
  async function saveCredential(id: string | null, draft: SettingsCredentialDraft): Promise<void> {
    const at = product.sent()
    let response: Response
    if (id === null) {
      const body: NewCredential = {
        name: draft.name,
        note: draft.note,
        keep: draft.keep ?? 'always',
        // A new credential has no saved value to keep: the editor requires every one.
        fields: draft.fields.map((field) => ({ ...field, value: field.value ?? '' })),
      }
      response = await apiRequest('/credentials', { method: 'POST', ...jsonBody(body) })
    } else {
      const body: CredentialPatch = {
        name: draft.name,
        note: draft.note,
        fields: draft.fields.map(({ value, ...field }) => (value === null ? field : { ...field, value })),
        ...(draft.keep === null ? {} : { keep: draft.keep }),
      }
      response = await apiRequest(`/credentials/${encodeURIComponent(id)}`, { method: 'PATCH', ...jsonBody(body) })
    }
    const { credential } = await readResponse(response, credentialAnswerSchema)
    product.answered(at, credentialsAnswer((current) => withCredential(current, credential)))
  }

  async function deleteCredential(id: string): Promise<void> {
    const at = product.sent()
    await apiRequest(`/credentials/${encodeURIComponent(id)}`, { method: 'DELETE' })
    product.answered(at, credentialsAnswer((current) => current.filter((credential) => credential.id !== id)))
  }

  /** The values of the credential's fields by name, for the eye of a secret field; null, after telling the user, when they could not be read. */
  async function revealCredential(id: string): Promise<Record<string, string> | null> {
    try {
      const response = await apiRequest(`/credentials/${encodeURIComponent(id)}/values`)
      return (await readResponse(response, credentialValuesSchema)).values
    } catch (error) {
      reportError(t('credentials.showFailed'), error, { userVisible: true })
      return null
    }
  }

  return { credentials, saveCredential, deleteCredential, revealCredential }
})

import { computed } from 'vue'
import { defineStore } from 'pinia'
import type { SettingsCredential, SettingsCredentialDraft } from '@demicodes/web-ui/settings/types'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import { credentialAnswerSchema, type CredentialDto, type NewCredential } from '../api/generated/web-api'
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
 * The user's credentials as Settings › Credentials changes them: a created or
 * deleted credential shows its answer, and the synchronization channel's
 * `credentials` part brings the same to every other page, each visible field
 * with its value and never a secret one's, which no route answers a page.
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

  /** Creates a credential, kept always as one the user adds is; a refusal is thrown for New Credential to show. */
  async function createCredential(draft: SettingsCredentialDraft): Promise<void> {
    const at = product.sent()
    const body: NewCredential = { name: draft.name, note: draft.note, keep: 'always', fields: draft.fields }
    const response = await apiRequest('/credentials', { method: 'POST', ...jsonBody(body) })
    const { credential } = await readResponse(response, credentialAnswerSchema)
    product.answered(at, credentialsAnswer((current) => withCredential(current, credential)))
  }

  async function deleteCredential(id: string): Promise<void> {
    const at = product.sent()
    await apiRequest(`/credentials/${encodeURIComponent(id)}`, { method: 'DELETE' })
    product.answered(at, credentialsAnswer((current) => current.filter((credential) => credential.id !== id)))
  }

  return { credentials, createCredential, deleteCredential }
})

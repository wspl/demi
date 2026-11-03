import { computed } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { CredentialReveal, MessageCredentials } from '@demicodes/web-ui/markdown/types'
import type { SettingsCredential, SettingsCredentialDraft, SettingsCredentialPatch } from '@demicodes/web-ui/settings/types'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  credentialAnswerSchema,
  revealedValueSchema,
  type CredentialDto,
  type CredentialPatch,
  type NewCredential,
} from '../api/generated/web-api'
import { confirmationHost } from '../auth/handoff'
import { useResources } from '../state/resources'
import { useHeldList, useProduct, type Answer } from '../state/product'
import { t } from './locales/text'

/** The credentials in name order, as the backend lists them: `credential` in its place by id. */
function withCredential(credentials: CredentialDto[], credential: CredentialDto): CredentialDto[] {
  const others = credentials.filter((candidate) => candidate.id !== credential.id)
  return [...others, credential].sort((a, b) => a.name.localeCompare(b.name))
}

/** The answer that leaves the credentials as `next` makes them of those the page last read. */
function credentialsAnswer(next: (current: CredentialDto[]) => CredentialDto[]): Answer {
  return (read) => ({ type: 'credentials', credentials: next(read.credentials ?? []) })
}

/**
 * The user's credentials as Settings › Credentials changes them: a created,
 * changed or deleted credential shows its answer, and the synchronization channel's
 * `credentials` part brings the same to every other page, each visible field
 * with its value and never a secret one's, which a reply's card asks for
 * alone (`messageCredentials`).
 */
const useCredentialStore = defineStore('credential-settings', () => {
  const product = useProduct()
  const credentials = computed<SettingsCredential[]>(() =>
    (product.snapshot?.credentials ?? []).map((credential) => ({
      id: credential.id,
      name: credential.name,
      note: credential.note,
      fields: credential.fields.map(({ value, ...field }) => ({ ...field, value: value ?? null })),
      keepUntil: credential.keepUntil,
      lastUsedAt: credential.lastUsedAt,
      createdAt: credential.createdAt,
    })),
  )

  /** Creates a credential with no values yet, kept always as one the user adds is, and answers its id. */
  async function createCredential(draft: SettingsCredentialDraft): Promise<string> {
    const at = product.sent()
    const body: NewCredential = {
      name: draft.name,
      keep: 'always',
      fields: draft.fields.map((field) => ({ ...field, value: '' })),
    }
    const response = await apiRequest('/credentials', { method: 'POST', ...jsonBody(body) })
    const { credential } = await readResponse(response, credentialAnswerSchema)
    product.answered(at, credentialsAnswer((current) => withCredential(current, credential)))
    return credential.id
  }

  /** Changes a credential; a refusal is thrown, in words for the user. */
  async function changeCredential(id: string, patch: SettingsCredentialPatch): Promise<void> {
    const at = product.sent()
    const body: CredentialPatch = patch
    const response = await apiRequest(`/credentials/${encodeURIComponent(id)}`, { method: 'PATCH', ...jsonBody(body) })
    const { credential } = await readResponse(response, credentialAnswerSchema)
    product.answered(at, credentialsAnswer((current) => withCredential(current, credential)))
  }

  async function deleteCredential(id: string): Promise<void> {
    const at = product.sent()
    await apiRequest(`/credentials/${encodeURIComponent(id)}`, { method: 'DELETE' })
    product.answered(at, credentialsAnswer((current) => current.filter((credential) => credential.id !== id)))
  }

  return { credentials, createCredential, changeCredential, deleteCredential }
})

/**
 * The user's credentials as the cards of their replies read them: the list
 * Settings shows, held while a card shows, and a secret field's value,
 * which `POST /credentials/:id/fields/:field/reveal` answers a session
 * verified in the last five minutes. A name is found as the vault finds it,
 * whatever its case.
 */
export function messageCredentials(): MessageCredentials {
  const product = useProduct()
  const store = useCredentialStore()
  const resources = useResources()
  return {
    hold: () => product.hold('credentials'),
    find(name) {
      const wanted = name.toLowerCase()
      const found = store.credentials.find((credential) => credential.name.toLowerCase() === wanted)
      if (found)
        return { state: 'found', credential: found }
      const load = product.listLoad('credentials')
      return { state: load === 'ready' ? 'missing' : load }
    },
    async reveal(id, field): Promise<CredentialReveal> {
      try {
        const response = await apiRequest(
          `/credentials/${encodeURIComponent(id)}/fields/${encodeURIComponent(field)}/reveal`,
          { method: 'POST' },
        )
        const { value } = await readResponse(response, revealedValueSchema)
        return { kind: 'revealed', value }
      } catch (error) {
        if (error instanceof ApiError && error.code === 'verification_required')
          return { kind: 'needsVerification' }
        reportError(t('credentials.revealFailed'), error, { site: 'credentials.revealFailed', userVisible: true })
        return { kind: 'failed' }
      }
    },
    get verification() {
      return confirmationHost()
    },
    get email() {
      return resources.email
    },
  }
}

/** The credential settings, which hold the user's credentials while a view shows them. */
export function useCredentialSettings(): ReturnType<typeof useCredentialStore> {
  useHeldList('credentials')
  return useCredentialStore()
}

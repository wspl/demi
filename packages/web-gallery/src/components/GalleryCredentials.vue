<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsCredentials from '@demicodes/web-ui/settings/SettingsCredentials.vue'
import type {
  SettingsCredential,
  SettingsCredentialDraft,
  SettingsCredentialPatch,
} from '@demicodes/web-ui/settings/types'
import type { CredentialState } from '../fixtures/credentials'

/**
 * The Credentials section over a specimen's state:
 *
 * - Each write waits a beat, as the product's does.
 * - A name another credential has is refused with the backend's message,
 *   which the Name field shows.
 * - A visible field lists its value and a secret one never does, as the
 *   backend's list does; a field without a value is missing.
 */
const props = defineProps<{
  state: CredentialState
}>()

const timers = new Set<number>()

/** Resolves after a beat, as a request to the backend would. */
function beat(): Promise<void> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(() => {
      timers.delete(timer)
      resolve()
    }, 400)
    timers.add(timer)
  })
}
onBeforeUnmount(() => {
  for (const timer of timers) {
    window.clearTimeout(timer)
  }
})

function nameTaken(name: string, id: string | null): void {
  const other = props.state.credentials.find((credential) => credential.id !== id && credential.name.toLowerCase() === name.toLowerCase())
  if (other) {
    throw new Error(`Another credential is named “${other.name}”. Choose another name.`)
  }
}

function sortByName(): void {
  props.state.credentials.sort((a, b) => a.name.localeCompare(b.name))
}

async function createCredential(draft: SettingsCredentialDraft): Promise<string> {
  await beat()
  nameTaken(draft.name, null)
  const credential: SettingsCredential = {
    id: `cred-${Date.now()}`,
    name: draft.name,
    note: '',
    fields: draft.fields.map((field) => ({ ...field, value: null, missing: true })),
    keepUntil: null,
    lastUsedAt: null,
    createdAt: new Date().toISOString(),
  }
  props.state.credentials.push(credential)
  sortByName()
  return credential.id
}

const KEEP_MS = { '1h': 60 * 60_000, '1d': 24 * 60 * 60_000, '30d': 30 * 24 * 60 * 60_000 } as const

async function changeCredential(id: string, patch: SettingsCredentialPatch): Promise<void> {
  await beat()
  const credential = props.state.credentials.find((candidate) => candidate.id === id)
  if (!credential) {
    throw new Error('This credential no longer exists.')
  }
  if (patch.name !== undefined) {
    nameTaken(patch.name, id)
    credential.name = patch.name
  }
  if (patch.note !== undefined) {
    credential.note = patch.note
  }
  if (patch.fields) {
    credential.fields = patch.fields.map((field) => {
      const kept = credential.fields.find((old) => old.name === field.name)
      return kept ? { ...kept, label: field.label } : { ...field, value: null, missing: true }
    })
  }
  for (const [name, value] of Object.entries(patch.values ?? {})) {
    const field = credential.fields.find((candidate) => candidate.name === name)
    if (field) {
      field.missing = value === ''
      field.value = field.kind === 'visible' && value !== '' ? value : null
    }
  }
  if (patch.keep !== undefined) {
    credential.keepUntil = patch.keep === 'always' ? null : new Date(Date.now() + KEEP_MS[patch.keep]).toISOString()
  }
  sortByName()
}

async function deleteCredential(id: string) {
  await beat()
  props.state.credentials = props.state.credentials.filter((credential) => credential.id !== id)
}
</script>

<template>
  <SettingsCredentials
    :credentials="state.credentials"
    :overlay-store="appOverlayStore"
    :create-credential="createCredential"
    :change-credential="changeCredential"
    :delete-credential="deleteCredential"
  />
</template>

<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsCredentials from '@demicodes/web-ui/settings/SettingsCredentials.vue'
import type { SettingsCredential, SettingsCredentialDraft } from '@demicodes/web-ui/settings/types'
import { ahead } from '../fixtures/time'
import { DEMO_CREDENTIAL_VALUES, type CredentialState } from '../fixtures/credentials'

/**
 * The Credentials section over a specimen's state. Each write waits a beat,
 * as the product's does; a name another credential has is refused with the
 * backend's message, so the editor shows its error. A saved visible field
 * lists its value, as the backend's list does; the reveal reads the
 * fixture's values, and a value the user typed in the editor is kept for it.
 */
const props = defineProps<{
  state: CredentialState
}>()

const timers = new Set<number>()
/** The values the specimen holds for the credentials the user saved here. */
const typed = new Map<string, Record<string, string>>()

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

const KEEP_MS = { '1h': 60 * 60_000, '1d': 24 * 60 * 60_000, '30d': 30 * 24 * 60 * 60_000 } as const

async function saveCredential(id: string | null, draft: SettingsCredentialDraft) {
  await beat()
  if (props.state.credentials.some((credential) => credential.id !== id && credential.name.toLowerCase() === draft.name.toLowerCase())) {
    throw new Error(`Another credential is named “${draft.name}”. Choose another name.`)
  }
  const existing = props.state.credentials.find((credential) => credential.id === id)
  const before = id === null ? {} : (typed.get(id) ?? DEMO_CREDENTIAL_VALUES[id] ?? {})
  const values = Object.fromEntries(draft.fields.map((field) => [field.name, field.value ?? before[field.name] ?? '']))
  const fields = draft.fields.map(({ name, label, kind }) => ({
    name,
    label,
    kind,
    value: kind === 'visible' ? (values[name] ?? '') : null,
  }))
  const keepUntil =
    draft.keep === null ? (existing?.keepUntil ?? null) : draft.keep === 'always' ? null : ahead(KEEP_MS[draft.keep])
  if (existing) {
    Object.assign(existing, { name: draft.name, note: draft.note, fields, keepUntil })
    typed.set(existing.id, values)
  } else {
    const credential: SettingsCredential = {
      id: `cred-${Date.now()}`,
      name: draft.name,
      note: draft.note,
      fields,
      keepUntil,
      lastUsedAt: null,
    }
    props.state.credentials.push(credential)
    typed.set(credential.id, values)
  }
  props.state.credentials.sort((a, b) => a.name.localeCompare(b.name))
}

async function deleteCredential(id: string) {
  await beat()
  props.state.credentials = props.state.credentials.filter((credential) => credential.id !== id)
  typed.delete(id)
}

async function revealCredential(id: string) {
  await beat()
  return typed.get(id) ?? DEMO_CREDENTIAL_VALUES[id] ?? {}
}
</script>

<template>
  <SettingsCredentials
    :credentials="state.credentials"
    :overlay-store="appOverlayStore"
    :save-credential="saveCredential"
    :delete-credential="deleteCredential"
    :reveal-credential="revealCredential"
  />
</template>

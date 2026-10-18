<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsCredentials from '@demicodes/web-ui/settings/SettingsCredentials.vue'
import type {
  CredentialRevealOutcome,
  SettingsCredential,
  SettingsCredentialDraft,
} from '@demicodes/web-ui/settings/types'
import { DEMO_CREDENTIAL_VALUES, type CredentialState } from '../fixtures/credentials'
import { galleryVerification } from '../fixtures/handoff'

/**
 * The Credentials section over a specimen's state. Each write waits a beat,
 * as the product's does; a name another credential has is refused with the
 * backend's message, so the editor shows its error. A saved visible field
 * lists its value, as the backend's list does; the reveal reads the
 * fixture's values, and a value the user typed in the editor is kept for it.
 * The specimen starts unconfirmed, so the first eye asks Confirm It’s You
 * (any password but `wrong` confirms) and every eye in the next five
 * minutes asks nothing.
 */
const props = defineProps<{
  state: CredentialState
}>()

const timers = new Set<number>()
const verification = galleryVerification()
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
  const keepUntil = draft.keep === 'always' ? null : (existing?.keepUntil ?? null)
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

async function revealCredential(id: string): Promise<CredentialRevealOutcome> {
  await beat()
  if (!verification.verified()) {
    return { kind: 'needsVerification' }
  }
  return { kind: 'revealed', values: typed.get(id) ?? DEMO_CREDENTIAL_VALUES[id] ?? {} }
}
</script>

<template>
  <SettingsCredentials
    :credentials="state.credentials"
    :overlay-store="appOverlayStore"
    :save-credential="saveCredential"
    :delete-credential="deleteCredential"
    :reveal-credential="revealCredential"
    :verification="verification.host"
    email="alex@example.com"
  />
</template>

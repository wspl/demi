<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsCredentials from '@demicodes/web-ui/settings/SettingsCredentials.vue'
import type { SettingsCredential, SettingsCredentialDraft } from '@demicodes/web-ui/settings/types'
import type { CredentialState } from '../fixtures/credentials'

/**
 * The Credentials section over a specimen's state:
 *
 * - Each write waits a beat, as the product's does.
 * - A name another credential has is refused with the backend's message, so
 *   New Credential shows its error.
 * - A created credential lists a visible field's value and never a secret
 *   one's, as the backend's list does; it is kept always.
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

async function createCredential(draft: SettingsCredentialDraft) {
  await beat()
  if (props.state.credentials.some((credential) => credential.name.toLowerCase() === draft.name.toLowerCase())) {
    throw new Error(`Another credential is named “${draft.name}”. Choose another name.`)
  }
  const credential: SettingsCredential = {
    id: `cred-${Date.now()}`,
    name: draft.name,
    note: draft.note,
    fields: draft.fields.map(({ name, label, kind, value }) => ({
      name,
      label,
      kind,
      value: kind === 'visible' ? value : null,
    })),
    keepUntil: null,
    lastUsedAt: null,
  }
  props.state.credentials.push(credential)
  props.state.credentials.sort((a, b) => a.name.localeCompare(b.name))
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
    :delete-credential="deleteCredential"
  />
</template>

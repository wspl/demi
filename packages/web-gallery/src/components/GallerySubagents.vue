<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsSubagents from '@demicodes/web-ui/settings/SettingsSubagents.vue'
import type { SettingsSubagentDraft } from '@demicodes/web-ui/settings/types'
import { subagentModels, subagentProviders, type SubagentState } from '../fixtures/subagent-profiles'

/**
 * The Subagent section over a specimen's state. Each write waits a beat, as
 * the product's does; a save with the reserved name or a name another
 * profile has is refused with the backend's message, so the editor shows
 * its error.
 */
const props = defineProps<{
  state: SubagentState
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

// A switch shows at once, as the product's does while its write is on its way.
function switchSubagents(enabled: boolean) {
  props.state.enabled = enabled
}

function switchProfile(id: string, enabled: boolean) {
  const profile = props.state.profiles.find((candidate) => candidate.id === id)
  if (profile) {
    profile.enabled = enabled
  }
}

async function saveProfile(id: string | null, draft: SettingsSubagentDraft) {
  await beat()
  if (draft.name === 'default') {
    throw new Error('name: "default" is reserved for inheriting the parent')
  }
  if (props.state.profiles.some((profile) => profile.name === draft.name && profile.id !== id)) {
    throw new Error('Another of your subagent profiles has that name')
  }
  const existing = props.state.profiles.find((profile) => profile.id === id)
  if (existing) {
    Object.assign(existing, draft)
  } else {
    props.state.profiles.push({ ...draft, id: `profile-${Date.now()}`, enabled: true })
  }
  props.state.profiles.sort((a, b) => a.name.localeCompare(b.name))
}

async function deleteProfile(id: string) {
  await beat()
  props.state.profiles = props.state.profiles.filter((profile) => profile.id !== id)
}
</script>

<template>
  <SettingsSubagents
    :enabled="state.enabled"
    :profiles="state.profiles"
    :providers="subagentProviders"
    :models="subagentModels"
    catalog-ready
    :overlay-store="appOverlayStore"
    :save-profile="saveProfile"
    :delete-profile="deleteProfile"
    @switch="switchSubagents"
    @switch-profile="switchProfile"
  />
</template>

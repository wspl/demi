<script setup lang="ts">
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsPlugin } from './types'

/**
 * The backend's plugins, one row each with a switch that turns it on or off
 * for the user (`plugins.md` § A user's plugins). A switch shows the choice
 * it asked for while the host saves it; the list the host passes back is the
 * one that holds. The switch alone says whether a plugin is off; its row
 * reads at full strength either way.
 */
defineProps<{
  plugins: SettingsPlugin[]
  /** The plugins whose switch is being saved. */
  pending?: readonly string[]
}>()

const emit = defineEmits<{
  switch: [id: string, enabled: boolean]
}>()
</script>

<template>
  <SettingsPage
    title="Plugins"
    description="What the agent and this app can do. A change shows here at once; an open conversation offers to reload so its agent has the commands that are on."
  >
    <SettingsGroup>
      <SettingsRow
        v-for="plugin in plugins"
        :key="plugin.id"
        :label="plugin.name"
        :description="plugin.description"
      >
        <Switch
          :model-value="plugin.enabled"
          size="sm"
          class="ml-1"
          :disabled="pending?.includes(plugin.id)"
          :aria-label="`${plugin.name} ${plugin.enabled ? 'on' : 'off'}`"
          @update:model-value="emit('switch', plugin.id, $event)"
        />
      </SettingsRow>
      <div
        v-if="!plugins.length"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        No plugins.
      </div>
    </SettingsGroup>
  </SettingsPage>
</template>

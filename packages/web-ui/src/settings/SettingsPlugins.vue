<script setup lang="ts">
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsPlugin } from './types'

/**
 * The backend's plugins, one row each with a switch that turns it on or off
 * for the user (`crates/backend-plugins/src/lib.rs`). The host shows a switch's
 * choice at once in the list it passes back, and that list is the one that
 * holds. The switch alone says whether a plugin is off; its row reads at
 * full strength either way.
 */
defineProps<{
  plugins: SettingsPlugin[]
}>()

const emit = defineEmits<{
  switch: [id: string, enabled: boolean]
}>()
</script>

<template>
  <SettingsPage
    :title="t('plugins.title')"
    :description="t('plugins.description')"
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
          :aria-label="plugin.name"
          @update:model-value="emit('switch', plugin.id, $event)"
        />
      </SettingsRow>
      <div
        v-if="!plugins.length"
        class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
      >
        {{ t('plugins.none') }}
      </div>
    </SettingsGroup>
  </SettingsPage>
</template>

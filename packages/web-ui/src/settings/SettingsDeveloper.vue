<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Switch from '../ui/Switch.vue'
import Tag from '../ui/Tag.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsExperiment } from './types'

/** Diagnostics and features that are not finished. */
defineProps<{
  overlayStore: OverlayStore
  levels: string[]
  experiments: SettingsExperiment[]
}>()

const logLevel = defineModel<string>('logLevel', { required: true })

const emit = defineEmits<{
  downloadLogs: []
  toggleExperiment: [id: string, on: boolean]
}>()
</script>

<template>
  <SettingsPage title="Developer" description="Diagnostics and features that are not finished.">
    <SettingsGroup title="Logging">
      <SettingsRow label="Log level" description="Debug writes every provider request. Large.">
        <Dropdown size="sm" :overlay-store="overlayStore" variant="default" trigger-label="Log level">
          <template #trigger>{{ logLevel }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="level in levels" :key="level" :label="level" choice :is-selected="logLevel === level" @select="logLevel = level; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Download logs" description="Recent diagnostics as a text file.">
        <Button size="sm" @click="emit('downloadLogs')">Download</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="Experiments" description="May change or disappear. Feedback welcome.">
      <SettingsRow v-for="experiment in experiments" :key="experiment.id" :label="experiment.name" :description="experiment.description">
        <template #tags><Tag tone="accent">Beta</Tag></template>
        <Switch :model-value="experiment.on" @update:model-value="emit('toggleExperiment', experiment.id, $event)" />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>

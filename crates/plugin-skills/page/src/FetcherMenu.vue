<script setup lang="ts">
import { t } from './locales/text'
import { computed, type Component } from 'vue'
import { Server } from '@lucide/vue'
import type { OverlayStore } from '@demicodes/plugin-sdk'
import { Dropdown } from '@demicodes/plugin-sdk'
import { Menu } from '@demicodes/plugin-sdk'
import { MenuItem } from '@demicodes/plugin-sdk'
import { deviceGlyph, deviceStatusLabel } from '@demicodes/plugin-sdk'
import type { SettingsSkillDevice } from './types'

/**
 * What fetches a source: the server, or one of the user's paired devices,
 * which fetches with its own git, keys and network.
 *
 * Its devices are drawn as every device menu draws them (`hosts/HostChoices.vue`):
 * the device's mark with its state, its name, and at the row's end its state
 * when it is not online or a download moves. An offline device is listed but
 * cannot be chosen until it is back. The trigger shows the choice's icon too:
 * the server's, the device's mark, or the plain computer of a device no longer
 * paired.
 */
const props = defineProps<{
  overlayStore: OverlayStore
  devices: readonly SettingsSkillDevice[]
  /** The chosen device's id; null for the server. */
  fetcher: string | null
  disabled?: boolean
}>()

const emit = defineEmits<{
  choose: [fetcher: string | null]
}>()

const chosen = computed(() => props.devices.find((device) => device.id === props.fetcher))

const label = computed(() => {
  if (props.fetcher === null) {
    return t('fetcher.server')
  }
  return chosen.value?.name ?? t('fetcher.removed')
})

const icon = computed((): Component => {
  if (props.fetcher === null) {
    return Server
  }
  return chosen.value ? deviceGlyph(chosen.value.state, chosen.value.mark) : deviceGlyph(null, null)
})

/** The end of a device's row: its state while it is not online, or a download that moves. */
function note(device: SettingsSkillDevice): string | undefined {
  return device.state !== 'online' || device.downloadPercent != null ? deviceStatusLabel(device) : undefined
}
</script>

<template>
  <Dropdown
    size="sm"
    variant="default"
    :trigger-label="t('fetcher.label')"
    :overlay-store="overlayStore"
    :disabled="disabled"
    :icon="icon"
  >
    <template #trigger>{{ label }}</template>
    <template #content="{ close }">
      <Menu>
        <MenuItem
          :icon="Server"
          :label="t('fetcher.server')"
          choice
          :is-selected="fetcher === null"
          @select="emit('choose', null); close()"
        />
        <MenuItem
          v-for="device in devices"
          :key="device.id"
          :icon="deviceGlyph(device.state, device.mark)"
          :label="device.name"
          :value="note(device)"
          choice
          :is-selected="fetcher === device.id"
          :disabled="device.state !== 'online'"
          :disabled-reason="t('fetcher.offline', { name: device.name })"
          @select="emit('choose', device.id); close()"
        />
      </Menu>
    </template>
  </Dropdown>
</template>

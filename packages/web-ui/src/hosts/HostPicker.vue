<script setup lang="ts">
import { computed } from 'vue'
import { Cloud, Monitor, Plus } from '@lucide/vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { MenuListItem } from '@demicodes/web-ui/ui/menu-context'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import type { HostDeviceOption } from './types'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE } from '../devices/state'

const props = defineProps<{
  devices: HostDeviceOption[]
  includeCloud?: boolean
  selectedId?: string
  boundIds?: string[]
}>()
const emit = defineEmits<{
  select: [id: string];
  connect: []
}>()
const items = computed((): MenuListItem[] =>
  props.devices.map((device) => ({
    ...device,
    label: device.name,
    icon: Monitor,
    note: device.state === 'online' ? undefined : DEVICE_STATE_LABEL[device.state],
    indicator: DEVICE_STATE_TONE[device.state],
    indicatorLabel: DEVICE_STATE_LABEL[device.state],
    disabledReason: disabledReason(device),
  })),
)
function disabledReason(device: HostDeviceOption): SentenceText | undefined {
  if (device.id === props.selectedId) {
    return 'Current primary host.'
  }
  if (props.boundIds?.includes(device.id)) {
    return 'Already attached to this conversation.'
  }
  if (device.state === 'updating') {
    return 'This device is updating.'
  }
  if (device.state === 'offline') {
    return 'This device is offline.'
  }
  return undefined
}
</script>

<template>
  <Menu
    :items="items"
    :iconless="false"
    :selected-id="selectedId"
    :is-item-disabled="(item) => !!item.disabledReason"
    filterable
    filter-placeholder="Search hosts…"
    empty-text="No Hosts Found"
    @select="emit('select', $event)"
  >
    <template #header>
      <MenuItem
        v-if="includeCloud"
        :icon="Cloud"
        label="Cloud"
        :disabled="selectedId === 'cloud'"
        @select="emit('select', 'cloud')"
      />
      <MenuItem
        :icon="Plus"
        label="Add Device…"
        @select="emit('connect')"
      />
    </template>
  </Menu>
</template>

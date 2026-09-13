<script setup lang="ts">
import { computed } from 'vue'
import { Cloud, Monitor, Plus } from '@lucide/vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { HostDeviceOption } from './types'

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
const items = computed(() =>
  props.devices.map((device) => ({
    ...device,
    label: device.name,
    icon: Monitor,
    note: device.online ? undefined : 'offline',
    indicator: device.online ? 'success' as const : 'muted' as const,
    indicatorLabel: device.online ? 'Online' : 'Offline',
    disabledReason: disabledReason(device),
  })),
)
function disabledReason(device: HostDeviceOption): string | undefined {
  if (device.id === props.selectedId) {
    return 'Current main host.'
  }
  if (props.boundIds?.includes(device.id)) {
    return 'Already attached to this conversation.'
  }
  if (!device.online) {
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
    empty-text="No hosts found"
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
        label="Connect new device"
        @select="emit('connect')"
      />
    </template>
  </Menu>
</template>

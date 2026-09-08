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
const emit = defineEmits<{ select: [id: string]; connect: [] }>()
const items = computed(() =>
  props.devices.map((device) => ({
    ...device,
    label: device.name,
    icon: Monitor,
    note: device.online ? undefined : 'offline',
  })),
)
function disabled(device: HostDeviceOption) {
  return (
    device.id === props.selectedId ||
    props.boundIds?.includes(device.id) ||
    // An offline device cannot be chosen anywhere; it stays listed so its absence is visible.
    !device.online
  )
}
</script>

<template>
  <Menu
    :items="items"
    :iconless="false"
    :selected-id="selectedId"
    :is-item-disabled="disabled"
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
      <MenuItem :icon="Plus" label="Connect new device" @select="emit('connect')" />
    </template>
  </Menu>
</template>

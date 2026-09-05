<script setup lang="ts">
import { computed } from 'vue'
import { ChevronRight } from '@lucide/vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SidebarExtension } from './types'

/** The list behind a Plugins or Skills entry: each item toggles in place; managing goes to its own surface. */
const props = defineProps<{
  title: string
  items: SidebarExtension[]
  manageLabel: string
}>()

const emit = defineEmits<{
  toggle: [id: string, enabled: boolean]
  manage: []
}>()

const enabledCount = computed(() => props.items.filter((item) => item.enabled).length)
</script>

<template>
  <Menu iconless>
    <div class="flex h-7 items-center justify-between px-2 text-[11px] uppercase tracking-wide text-fg-subtle">
      <span>{{ title }}</span>
      <span>{{ enabledCount }} / {{ items.length }} on</span>
    </div>
    <MenuItem v-for="item in items" :key="item.id" @select="emit('toggle', item.id, !item.enabled)">
      <span class="flex min-w-0 flex-1 items-baseline gap-2">
        <span class="shrink-0" :class="item.enabled ? 'text-fg' : 'text-fg-muted'">{{ item.name }}</span>
        <span class="min-w-0 truncate text-[12px] text-fg-subtle">{{ item.summary }}</span>
      </span>
      <template #suffix>
        <Switch :model-value="item.enabled" size="sm" @click.stop @update:model-value="emit('toggle', item.id, $event)" />
      </template>
    </MenuItem>
    <MenuDivider />
    <MenuItem @select="emit('manage')">
      <span class="min-w-0 flex-1 truncate">{{ manageLabel }}</span>
      <template #suffix>
        <ChevronRight :size="ICON_PX.in28" class="text-fg-faint" />
      </template>
    </MenuItem>
  </Menu>
</template>

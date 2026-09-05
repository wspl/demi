<script setup lang="ts">
import { ChevronsDownUp, ChevronsUpDown, ListChecks, SquarePen, Trash2 } from '@lucide/vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { SidebarProject } from './types'

defineProps<{
  project: SidebarProject
  folded: boolean
  count: number
}>()

const emit = defineEmits<{
  create: []
  toggleFold: []
  selectAll: []
  remove: []
}>()
</script>

<template>
  <Menu>
    <MenuItem :icon="SquarePen" label="New conversation here" @select="emit('create')" />
    <MenuItem :icon="folded ? ChevronsUpDown : ChevronsDownUp" :label="folded ? 'Unfold' : 'Fold'" shortcut="↵" @select="emit('toggleFold')" />
    <MenuItem :icon="ListChecks" label="Select conversations" :disabled="count === 0" @select="emit('selectAll')" />
    <MenuDivider />
    <MenuItem :icon="Trash2" label="Remove project" is-danger @select="emit('remove')" />
  </Menu>
</template>

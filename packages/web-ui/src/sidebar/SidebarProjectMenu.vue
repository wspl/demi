<script setup lang="ts">
import { ChevronsDownUp, ChevronsUpDown, ListChecks, SquarePen, Trash2 } from '@lucide/vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { Action } from '@demicodes/web-ui/ui/pending-action'
import type { SidebarProject } from './types'

defineProps<{
  project: SidebarProject
  folded: boolean
  count: number
  /** Removes the project; its row works until it settles. */
  onRemove?: Action
}>()

const emit = defineEmits<{
  create: []
  toggleFold: []
  selectAll: []
}>()
</script>

<template>
  <MenuItem
    :icon="SquarePen"
    label="New Conversation Here"
    @select="emit('create')"
  />
  <MenuItem
    :icon="folded ? ChevronsUpDown : ChevronsDownUp"
    :label="folded ? 'Unfold' : 'Fold'"
    shortcut="↵"
    @select="emit('toggleFold')"
  />
  <MenuItem
    :icon="ListChecks"
    label="Select Conversations"
    :disabled="count === 0"
    @select="emit('selectAll')"
  />
  <MenuDivider />
  <MenuItem
    :icon="Trash2"
    label="Remove Project"
    is-danger
    @select="onRemove?.()"
  />
</template>

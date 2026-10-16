<script setup lang="ts">
import { t } from './locales/text'
import { ChevronsDownUp, ChevronsUpDown, ListChecks, SquarePen, Trash2 } from '@lucide/vue'
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
  /** Asks before the project is deleted (`ProjectDeleteDialog`). */
  delete: []
}>()
</script>

<template>
  <MenuItem
    :icon="SquarePen"
    :label="t('projectMenu.newHere')"
    @select="emit('create')"
  />
  <MenuItem
    :icon="folded ? ChevronsUpDown : ChevronsDownUp"
    :label="folded ? t('projectMenu.unfold') : t('projectMenu.fold')"
    shortcut="↵"
    @select="emit('toggleFold')"
  />
  <MenuItem
    :icon="ListChecks"
    :label="t('projectMenu.select')"
    :disabled="count === 0"
    @select="emit('selectAll')"
  />
  <MenuDivider />
  <MenuItem
    :icon="Trash2"
    :label="t('projectMenu.delete')"
    is-danger
    @select="emit('delete')"
  />
</template>

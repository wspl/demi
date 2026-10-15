<script setup lang="ts">
import { t } from './locales/text'
import { ChevronsDownUp, ChevronsUpDown, ListChecks, SquarePen, Trash2 } from '@lucide/vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { Action } from '@demicodes/web-ui/ui/operations'
import { projectTarget } from './project-hosts'
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
    :label="t('projectMenu.remove')"
    is-danger
    :operation="{ action: 'remove', target: projectTarget(project.id) }"
    @select="onRemove?.()"
  />
</template>

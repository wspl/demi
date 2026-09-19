<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import Tooltip from '../ui/Tooltip.vue'
import FileIcon from './FileIcon.vue'
import { isHiddenName } from './paths'
import type { TreeRow } from './tree'

/**
 * One row of a `Tree`, in the tree or pinned in its sticky stack: a chevron
 * for a directory, the file icon, the name. A tree dresses its rows through
 * the slots: `mark` sits on the icon's corner, `name` replaces the plain
 * name, `trailing` ends the row, kept off the scrollbar. A `tooltip` covers
 * the whole row. A hidden entry's icon and name are faded. A row whose menu
 * is open keeps its hover look until the menu closes.
 */
defineProps<{
  row: TreeRow
  selected: boolean
  menuOpen?: boolean
  tooltip?: string
}>()

defineEmits<{
  activate: []
}>()
</script>

<template>
  <div
    role="treeitem"
    :aria-selected="selected"
    :aria-expanded="row.isDirectory ? row.open : undefined"
    class="flex h-7 shrink-0 cursor-default select-none items-center rounded-md pr-2 text-chrome transition-colors duration-200 ease-out"
    :class="selected ? 'bg-active text-fg-emphasis' : menuOpen ? 'bg-hover text-fg' : 'text-fg-body hover:bg-hover'"
    :style="{ paddingLeft: `${4 + row.depth * 12}px` }"
    @click="$emit('activate')"
  >
    <Tooltip
      tag="span"
      class="flex h-full min-w-0 flex-1 items-center gap-1"
      :content="tooltip ?? ''"
      :disabled="!tooltip"
      placement="bottom"
    >
      <!-- Directories fold on a chevron; files keep its width so names line up. -->
      <span class="flex size-4 shrink-0 items-center justify-center text-fg-faint">
        <ChevronRight
          v-if="row.isDirectory"
          :size="ICON_PX.in20"
          class="transition-transform duration-150"
          :class="row.open ? 'rotate-90' : ''"
        />
      </span>
      <span class="relative inline-flex shrink-0">
        <FileIcon :name="row.name" :is-directory="row.isDirectory" :class="isHiddenName(row.name) ? 'faded' : ''" />
        <slot name="mark" />
      </span>
      <span class="flex min-w-0" :class="isHiddenName(row.name) ? 'faded' : ''">
        <slot name="name">
          <span class="truncate">{{ row.name }}</span>
        </slot>
      </span>
      <slot name="trailing" />
    </Tooltip>
  </div>
</template>

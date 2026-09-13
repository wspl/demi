<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import type { FileTreeRow } from './file-tree'
import type { FileBrowserFailure } from './types'

/** One row of a `FileTree`, in the tree or pinned in its sticky stack. */
defineProps<{
  row: FileTreeRow
  selected: boolean
  open: boolean
  loading: boolean
  failure: FileBrowserFailure | null
}>()

defineEmits<{
  activate: []
}>()
</script>

<template>
  <div
    role="treeitem"
    :aria-selected="selected"
    :aria-expanded="row.isDirectory ? open : undefined"
    class="flex h-7 shrink-0 cursor-default select-none items-center gap-1 rounded-md pr-1 text-chrome transition-colors duration-200 ease-out"
    :class="selected ? 'bg-active text-fg-emphasis' : 'text-fg-body hover:bg-hover'"
    :style="{ paddingLeft: `${4 + row.depth * 12}px` }"
    :title="row.name"
    @click="$emit('activate')"
  >
    <!-- Directories fold on a chevron; files keep its width so names line up. -->
    <span class="flex size-4 shrink-0 items-center justify-center text-fg-faint">
      <ChevronRight
        v-if="row.isDirectory"
        :size="ICON_PX.in20"
        class="transition-transform duration-150"
        :class="open ? 'rotate-90' : ''"
      />
    </span>
    <FileIcon :name="row.name" :is-directory="row.isDirectory" />
    <span class="truncate">{{ row.name }}</span>
    <!-- A listing in flight: a thin spinner at the row's end, the chevron untouched. -->
    <IndeterminateSpinner
      v-if="loading"
      class="ml-auto mr-1 shrink-0 text-fg-faint"
      :size="12"
      :stroke-width="1.5"
    />
    <span
      v-else-if="failure"
      class="ml-auto truncate pl-2 text-[11px] text-fg-faint"
      :title="failure.message"
      >{{ failure.kind === 'permission' ? 'No access' : 'Unavailable' }}</span
    >
  </div>
</template>

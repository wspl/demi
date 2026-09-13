<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { computed } from 'vue'
import CornerDot from '../ui/CornerDot.vue'
import Tooltip from '../ui/Tooltip.vue'
import FileIcon from './FileIcon.vue'
import type { FileTreeRow } from './file-tree'
import type { FileBrowserFailure } from './types'

/** One row of a `FileTree`, in the tree or pinned in its sticky stack. */
const props = defineProps<{
  row: FileTreeRow
  selected: boolean
  open: boolean
  loading: boolean
  failure: FileBrowserFailure | null
}>()

defineEmits<{
  activate: []
}>()

/** Why the directory could not be listed, for the tooltip on its dot. */
const failureText = computed(() => {
  const failure = props.failure
  if (!failure) {
    return ''
  }
  const heading = failure.kind === 'permission' ? 'No access' : 'Unavailable'
  return failure.message ? `${heading}: ${failure.message}` : heading
})
</script>

<template>
  <div
    role="treeitem"
    :aria-selected="selected"
    :aria-expanded="row.isDirectory ? open : undefined"
    class="flex h-7 shrink-0 cursor-default select-none items-center gap-1 rounded-md pr-1 text-chrome transition-colors duration-200 ease-out"
    :class="selected ? 'bg-active text-fg-emphasis' : 'text-fg-body hover:bg-hover'"
    :style="{ paddingLeft: `${4 + row.depth * 12}px` }"
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
    <!-- A directory that could not be listed wears a red dot; hovering the icon tells why. -->
    <Tooltip
      tag="span"
      class="relative inline-flex shrink-0"
      :content="failureText"
      :disabled="!failure"
      placement="bottom"
    >
      <FileIcon :name="row.name" :is-directory="row.isDirectory" />
      <CornerDot v-if="failure" tone="danger" size="xs" ring="editor" :label="failureText" />
    </Tooltip>
    <span class="truncate">{{ row.name }}</span>
    <!-- A listing in flight: a thin spinner at the row's end, the chevron untouched. -->
    <IndeterminateSpinner
      v-if="loading"
      class="ml-auto mr-1 shrink-0 text-fg-faint"
      :size="12"
      :stroke-width="1.5"
    />
  </div>
</template>

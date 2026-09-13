<script setup lang="ts">
import { computed, ref } from 'vue'
import { ChevronRight } from '@lucide/vue'
import type { ShellFileChange } from '@demicodes/agent'
import CornerDot from '../ui/CornerDot.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from './FileIcon.vue'
import { changeTreeRows } from './changes'
import { baseName } from './paths'

/**
 * The changed files as a tree beside the diff: directories on the way to
 * them fold and unfold, each file shows how it changed the way the
 * changed-file pills do (a green dot for a new file, a struck name for a
 * deleted one) and its line counts at the row's end, kept clear of the
 * overlay scrollbar. A click on a file selects it.
 */
const props = defineProps<{
  files: readonly ShellFileChange[]
  /** The workspace, named at the top. */
  root: string
  /** The selected file, by path relative to the workspace. */
  selected: string | null
}>()

const emit = defineEmits<{
  select: [path: string]
}>()

const folded = ref(new Set<string>())
const rows = computed(() => changeTreeRows(props.files, folded.value))
const rootName = computed(() => baseName(props.root) || '/')

function toggle(path: string): void {
  const next = new Set(folded.value)
  if (next.has(path)) {
    next.delete(path)
  } else {
    next.add(path)
  }
  folded.value = next
}
</script>

<template>
  <ScrollArea class="h-full min-h-0 bg-surface-editor" viewport-class="p-1">
    <div class="flex h-7 shrink-0 select-none items-center px-2 text-chrome font-medium text-fg-muted" :title="root">
      <span class="truncate">{{ rootName }}</span>
    </div>
    <div role="tree" :aria-label="rootName" class="flex min-h-full flex-col gap-px">
      <div
        v-for="row in rows"
        :key="row.path"
        role="treeitem"
        :aria-selected="row.kind === 'file' && row.path === selected"
        :aria-expanded="row.kind === 'directory' ? !folded.has(row.path) : undefined"
        class="flex h-7 shrink-0 cursor-default select-none items-center gap-1 rounded-md pr-3 text-chrome transition-colors duration-200 ease-out"
        :class="row.kind === 'file' && row.path === selected ? 'bg-active text-fg-emphasis' : 'text-fg-body hover:bg-hover'"
        :style="{ paddingLeft: `${4 + row.depth * 12}px` }"
        @click="row.kind === 'directory' ? toggle(row.path) : emit('select', row.path)"
      >
        <span class="flex size-4 shrink-0 items-center justify-center text-fg-faint">
          <ChevronRight
            v-if="row.kind === 'directory'"
            :size="ICON_PX.in20"
            class="transition-transform duration-150"
            :class="folded.has(row.path) ? '' : 'rotate-90'"
          />
        </span>
        <span class="relative inline-flex shrink-0">
          <FileIcon :name="row.name" :is-directory="row.kind === 'directory'" />
          <CornerDot v-if="row.kind === 'file' && row.change.kind === 'added'" tone="success" size="xs" ring="editor" />
        </span>
        <span class="truncate" :class="row.kind === 'file' && row.change.kind === 'deleted' ? 'line-through text-fg-muted' : ''">{{ row.name }}</span>
        <!-- Counts as on the changed-file pills: added in green, removed in red, zero omitted. -->
        <span v-if="row.kind === 'file'" class="ml-auto inline-flex shrink-0 gap-1 pl-2 font-mono text-[11px] tabular-nums">
          <span v-if="row.change.added > 0" class="text-on-success">+{{ row.change.added }}</span>
          <span v-if="row.change.removed > 0" class="text-on-danger">−{{ row.change.removed }}</span>
        </span>
      </div>
      <div v-if="rows.length === 0" class="flex flex-1 select-none items-center justify-center py-10 text-[13px] text-fg-subtle">
        No changes
      </div>
    </div>
  </ScrollArea>
</template>

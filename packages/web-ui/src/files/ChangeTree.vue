<script setup lang="ts">
import { computed, ref } from 'vue'
import type { ShellFileChange } from '@demicodes/agent'
import CornerDot from '../ui/CornerDot.vue'
import Tree from './Tree.vue'
import { changeTotals, changeTreeRows, type ChangeTreeRow } from './changes'
import { baseName } from './paths'

/**
 * The changed files as a tree beside the diff, on a `Tree`: directories on
 * the way to them fold and unfold, each file shows how it changed the way
 * the changed-file pills do (a green dot for a new file, a struck name for
 * a deleted one) and its line counts at the row's end; the caption row
 * counts the files and lines. A click on a file selects it.
 */
const props = defineProps<{
  files: readonly ShellFileChange[]
  /** The workspace, named at the top. */
  root: string
  /** The selected file, by path relative to the workspace. */
  selected: string | null
  /** What the tree says when there are no files. */
  emptyText: string
}>()

const emit = defineEmits<{
  select: [path: string]
}>()

const folded = ref(new Set<string>())
const rows = computed(() => changeTreeRows(props.files, folded.value))
const rootName = computed(() => baseName(props.root) || '/')
const totals = computed(() => changeTotals(props.files))

function toggle(path: string): void {
  const next = new Set(folded.value)
  if (next.has(path)) {
    next.delete(path)
  } else {
    next.add(path)
  }
  folded.value = next
}

function activate(row: ChangeTreeRow): void {
  if (row.isDirectory) {
    toggle(row.path)
  } else {
    emit('select', row.path)
  }
}
</script>

<template>
  <Tree
    :rows="rows"
    :caption="rootName"
    :caption-title="root"
    :selected="selected"
    @activate="activate"
  >
    <template #captionTrailing>
      <span v-if="files.length > 0" class="ml-2 inline-flex shrink-0 gap-1 font-mono text-[11px] font-normal tabular-nums">
        <span>{{ files.length }} {{ files.length === 1 ? 'file' : 'files' }}</span>
        <span v-if="totals.added > 0" class="text-on-success">+{{ totals.added }}</span>
        <span v-if="totals.removed > 0" class="text-on-danger">−{{ totals.removed }}</span>
      </span>
    </template>
    <template #mark="{ row }">
      <CornerDot v-if="row.change?.kind === 'added'" tone="success" size="xs" ring="editor" />
    </template>
    <template #name="{ row }">
      <span class="truncate" :class="row.change?.kind === 'deleted' ? 'line-through text-fg-muted' : ''">{{ row.name }}</span>
    </template>
    <template #trailing="{ row }">
      <!-- Counts as on the changed-file pills: added in green, removed in red, zero omitted. -->
      <span v-if="row.change" class="ml-auto mr-1 inline-flex shrink-0 gap-1 pl-2 font-mono text-[11px] tabular-nums">
        <span v-if="row.change.added > 0" class="text-on-success">+{{ row.change.added }}</span>
        <span v-if="row.change.removed > 0" class="text-on-danger">−{{ row.change.removed }}</span>
      </span>
    </template>
    <template #empty>
      <div class="flex flex-1 select-none items-center justify-center px-4 py-10 text-center text-[13px] text-fg-subtle">
        {{ emptyText }}
      </div>
    </template>
  </Tree>
</template>

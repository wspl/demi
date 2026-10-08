<script setup lang="ts">
import { computed } from 'vue'
import { baseName } from '@demicodes/utils'
import HighlightText from '../ui/HighlightText.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import Tree from './Tree.vue'
import { relativePath, resolveHostPath } from './paths'
import type { RequestFile } from './request-changes'
import type { TreeRow } from './tree'

/**
 * A request's files beside the diff, in the order the request first changed
 * them, on a `Tree` of one level: each row the file's name, its folder from
 * the workspace after it, as VS Code's source control list shows them,
 * without line counts: only the diff a file shows can tell them right, and
 * the view's header gives them.
 */
const props = defineProps<{
  files: readonly RequestFile[]
  /** The workspace, named at the top. */
  root: string
  /** What heads the list in place of the root directory's name. */
  rootName?: string
  /** The selected file, by its path. */
  selected: string | null
}>()

const emit = defineEmits<{
  select: [path: string]
}>()

interface RequestRow extends TreeRow {
  file: RequestFile
  folder: string
}

const rows = computed<RequestRow[]>(() => props.files.map((file) => {
  const shown = relativePath(props.root, resolveHostPath(props.root, file.path))
  return {
    path: file.path,
    name: baseName(file.path),
    isDirectory: false,
    depth: 0,
    parent: null,
    open: false,
    file,
    folder: shown.slice(0, Math.max(0, shown.lastIndexOf('/'))),
  }
}))
const rootName = computed(() => props.rootName ?? (baseName(props.root) || '/'))
</script>

<template>
  <Tree
    :rows="rows"
    :caption="rootName"
    :caption-title="root"
    :selected="selected"
    :tooltip="(row) => row.file.path"
    @activate="emit('select', $event.path)"
  >
    <template #name="{ row, highlight }">
      <!-- The name stays whole; the folder after it gives way first (the gallery's Writing page). -->
      <span class="flex min-w-0 items-baseline gap-1.5">
        <span class="shrink-0"><HighlightText :text="row.name" :indexes="highlight" /></span>
        <TruncatedText v-if="row.folder" class="text-[11px] text-fg-subtle" :text="row.folder" />
      </span>
    </template>
    <template #empty>
      <div class="flex flex-1 select-none items-center justify-center px-4 py-10 text-center text-[13px] text-fg-subtle">
        These changes are no longer in the conversation.
      </div>
    </template>
  </Tree>
</template>

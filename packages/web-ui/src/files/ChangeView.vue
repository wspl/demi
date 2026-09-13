<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { FileOutput, FolderTree } from '@lucide/vue'
import DiffEditor from '../editor/components/DiffEditor.vue'
import { appEditorHost } from '../editor/host/appHost'
import { toEditorUri } from '../editor/editorUri'
import IconButton from '../ui/IconButton.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import Tooltip from '../ui/Tooltip.vue'
import ChangeTree from './ChangeTree.vue'
import { changeTotals, type ChangeSetSource } from './changes'
import { TREE_WIDTH } from './file-view'
import { FileBrowserError } from './types'
import { joinPath } from './paths'

/**
 * The conversation's changes: the diff of the selected file, read through
 * the change set, and beside it the tree of changed files with their kinds
 * and line counts. The row above says how many files and lines changed and
 * holds the control that opens the selected file itself (not for a deleted
 * one) and the one that shows and hides the tree; the host keeps the tree's
 * visibility and width (v-model) as it does for the file view, and which file
 * is selected.
 */
const props = defineProps<{
  changes: ChangeSetSource
  /** The workspace the paths are relative to. */
  root: string
  /** Fold the unchanged stretches between changes in the diff; off shows whole files. */
  collapseUnchanged?: boolean
}>()

const emit = defineEmits<{
  /** Open the selected file itself, by its path relative to the workspace. */
  open: [path: string]
}>()

const selected = defineModel<string | null>('selected', { default: null })
const tree = defineModel<boolean>('tree', { default: true })
const treeWidth = defineModel<number>('treeWidth', { default: TREE_WIDTH.default })

const totals = computed(() => changeTotals(props.changes.files))
const selectedChange = computed(
  () => props.changes.files.find((file) => file.path === selected.value) ?? null,
)

const sides = ref<{ original: string; modified: string } | null>(null)
const state = ref<'idle' | 'loading' | 'ready' | 'failed'>('idle')
const failure = ref<string | null>(null)
let controller: AbortController | null = null

async function read(): Promise<void> {
  controller?.abort()
  const path = selected.value
  if (path === null) {
    state.value = 'idle'
    sides.value = null
    return
  }
  const current = new AbortController()
  controller = current
  state.value = 'loading'
  failure.value = null
  try {
    const result = await props.changes.read(path, current.signal)
    if (current.signal.aborted) {
      return
    }
    sides.value = result
    state.value = 'ready'
  } catch (error) {
    if (current.signal.aborted) {
      return
    }
    state.value = 'failed'
    failure.value = error instanceof FileBrowserError || error instanceof Error ? error.message : String(error)
  }
}

watch(() => [props.changes, selected.value], read, { immediate: true })

onBeforeUnmount(() => {
  controller?.abort()
})
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="flex h-11 shrink-0 items-center gap-2 px-3">
      <span class="min-w-0 flex-1 truncate text-chrome text-fg-muted">
        {{ changes.files.length }} {{ changes.files.length === 1 ? 'file' : 'files' }} changed
        <span class="ml-1 font-mono text-[11px] tabular-nums">
          <span v-if="totals.added > 0" class="text-on-success">+{{ totals.added }}</span>
          <span v-if="totals.removed > 0" class="ml-1 text-on-danger">−{{ totals.removed }}</span>
        </span>
      </span>
      <Tooltip content="Open file" class="shrink-0">
        <IconButton
          :icon="FileOutput"
          variant="ghost"
          aria-label="Open file"
          :disabled="!selectedChange || selectedChange.kind === 'deleted'"
          @click="selectedChange && emit('open', selectedChange.path)"
        />
      </Tooltip>
      <Tooltip :content="tree ? 'Hide changed files' : 'Show changed files'" class="shrink-0">
        <IconButton
          :icon="FolderTree"
          variant="ghost"
          :pressed="tree"
          :aria-label="tree ? 'Hide changed files' : 'Show changed files'"
          @click="tree = !tree"
        />
      </Tooltip>
    </div>
    <div class="flex min-h-0 flex-1 border-t border-line">
      <div class="relative min-w-0 flex-1">
        <!-- A diff is built for one pair of texts: a new file is a new editor. -->
        <DiffEditor
          v-if="state === 'ready' && sides && selectedChange"
          :key="selectedChange.path"
          :host="appEditorHost"
          :original="sides.original"
          :modified="sides.modified"
          :filename="selectedChange.path"
          :resource-uri="toEditorUri('workspace', joinPath(root, selectedChange.path))"
          :collapse-unchanged="collapseUnchanged"
        />
        <RegionStatus
          v-else
          class="h-full"
          :busy="state === 'loading'"
          :failed="state === 'failed'"
          :label="state === 'loading' ? 'Reading…' : state === 'failed' ? 'Could not read this change.' : 'Select a changed file.'"
          :detail="failure"
          :action="state === 'failed' ? 'Retry' : undefined"
          @action="read"
        />
      </div>
      <template v-if="tree">
        <ResizeHandle
          v-model="treeWidth"
          side="end"
          :min="TREE_WIDTH.min"
          :max="TREE_WIDTH.max"
          :default-value="TREE_WIDTH.default"
          label="Changed files width"
        />
        <ChangeTree
          class="border-l border-line"
          :style="{ flex: `0 0 ${treeWidth}px`, width: `${treeWidth}px` }"
          :files="changes.files"
          :root="root"
          :selected="selected"
          @select="selected = $event"
        />
      </template>
    </div>
  </div>
</template>

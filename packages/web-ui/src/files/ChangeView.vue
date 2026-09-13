<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, FileOutput, FolderTree } from '@lucide/vue'
import DiffEditor from '../editor/components/DiffEditor.vue'
import { appEditorHost } from '../editor/host/appHost'
import { toEditorUri } from '../editor/editorUri'
import IconButton from '../ui/IconButton.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tooltip from '../ui/Tooltip.vue'
import ChangeTree from './ChangeTree.vue'
import { emptyChangeSetText, type ChangeMode, type ChangeSources } from './changes'
import { TREE_WIDTH } from './file-view'
import { FileBrowserError } from './types'
import { joinPath } from './paths'

/**
 * The changes as diffs, from one of two sources the switch in the header
 * picks between: files picked from the conversation, each a snapshot around
 * one tool call, or the workspace's uncommitted changes. Uncommitted shows
 * the diff of the selected file beside the tree of changed files, with
 * their kinds and line counts, the files and lines summed up in the tree's
 * caption; Conversation is one picked file at a time, no tree. Either way
 * the header names the file shown, with its counts. Back and Forward walk
 * what the view has shown, across modes; the
 * host keeps that history (`showChangeInTab`) along with the mode, the
 * selected file, and the tree's visibility and width. The header also holds
 * the control that opens the selected file itself (not a deleted one) and,
 * under Uncommitted, the one that shows and hides the tree; the tree's own
 * caption lists the changes again. With nothing to list, the view says why:
 * nothing changed, no repository, or a listing that failed.
 */
const props = defineProps<{
  changes: ChangeSources
  /** The workspace the paths are relative to. */
  root: string
  canBack?: boolean
  canForward?: boolean
  /** Fold the unchanged stretches between changes in the diff; off shows whole files. */
  collapseUnchanged?: boolean
}>()

const emit = defineEmits<{
  /** Open the selected file itself, by its path relative to the workspace. */
  open: [path: string]
  back: []
  forward: []
}>()

const mode = defineModel<ChangeMode>('mode', { required: true })
const selected = defineModel<string | null>('selected', { default: null })
const tree = defineModel<boolean>('tree', { default: true })
const treeWidth = defineModel<number>('treeWidth', { default: TREE_WIDTH.default })

const modeOptions: readonly SegmentedOption<ChangeMode>[] = [
  { value: 'uncommitted', label: 'Uncommitted' },
  { value: 'conversation', label: 'Conversation' },
]

const source = computed(() => props.changes[mode.value])
const selectedChange = computed(
  () => source.value.files.find((file) => file.path === selected.value) ?? null,
)
const treeShown = computed(() => mode.value === 'uncommitted' && tree.value)

const emptyText = computed(() => emptyChangeSetText(source.value, mode.value))
const idleText = computed(() => (source.value.files.length > 0 ? 'Select a changed file.' : emptyText.value))

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
    const result = await source.value.read(path, current.signal)
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

watch(() => [source.value, selected.value], read, { immediate: true })

onBeforeUnmount(() => {
  controller?.abort()
})
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <!-- Back and Forward move through what this view has shown, mode and file. -->
      <div class="flex shrink-0 items-center">
        <Tooltip content="Back">
          <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Back" :disabled="!canBack" @click="emit('back')" />
        </Tooltip>
        <Tooltip content="Forward">
          <IconButton :icon="ArrowRight" variant="ghost" aria-label="Forward" :disabled="!canForward" @click="emit('forward')" />
        </Tooltip>
      </div>
      <Segmented v-model="mode" :options="modeOptions" size="sm" class="ml-1 shrink-0" />
      <!-- The right side, whether or not a file is shown: its name and counts, then the controls. -->
      <div class="ml-auto flex min-w-0 items-center gap-1">
        <span v-if="selectedChange" class="flex min-w-0 items-center gap-1 pl-2 font-mono text-[11px] text-fg-muted">
          <span class="truncate" :class="selectedChange.kind === 'deleted' ? 'line-through' : ''" :title="selectedChange.path">{{ selectedChange.path }}</span>
          <span class="shrink-0 tabular-nums">
            <span v-if="selectedChange.added > 0" class="text-on-success">+{{ selectedChange.added }}</span>
            <span v-if="selectedChange.removed > 0" class="ml-1 text-on-danger">−{{ selectedChange.removed }}</span>
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
        <Tooltip v-if="mode === 'uncommitted'" :content="tree ? 'Hide changed files' : 'Show changed files'" class="shrink-0">
          <IconButton
            :icon="FolderTree"
            variant="ghost"
            :pressed="tree"
            :aria-label="tree ? 'Hide changed files' : 'Show changed files'"
            @click="tree = !tree"
          />
        </Tooltip>
      </div>
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
          :label="state === 'loading' ? 'Reading…' : state === 'failed' ? 'Could not read this change.' : idleText"
          :detail="failure"
          :action="state === 'failed' ? 'Retry' : undefined"
          @action="read"
        />
      </div>
      <template v-if="treeShown">
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
          :source="source"
          :root="root"
          :selected="selected"
          :empty-text="emptyText"
          @select="selected = $event"
        />
      </template>
    </div>
  </div>
</template>

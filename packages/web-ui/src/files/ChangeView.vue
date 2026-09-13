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
import { changeAbsolutePath, changeDisplayPath, emptyChangeSetText, type ChangeMode, type ChangeSources } from './changes'
import { TREE_WIDTH } from './file-view'

/**
 * One working-tree file or one retained segment of a call. The shared tree,
 * segment control and Back/Forward events use the selection held by the host.
 * A missing retained pair leaves the editor area empty.
 */
const props = defineProps<{
  changes: ChangeSources
  /** The workspace the paths are relative to. */
  root: string
  /** What the tree's caption says in place of the root directory's name. */
  rootName?: string
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
const edit = defineModel<number>('edit', { default: 0 })
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
const segments = computed(() => source.value.segments?.(selected.value ?? '') ?? [])
const displayPath = computed(() => changeDisplayPath(selected.value ?? '', props.root))

const emptyText = computed(() => emptyChangeSetText(source.value, mode.value))
const idleText = computed(() => (source.value.files.length > 0 ? 'Select a changed file.' : emptyText.value))

const sides = ref<{ original: string; modified: string } | null>(null)
const state = ref<'idle' | 'loading' | 'ready' | 'failed' | 'unavailable'>('idle')
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
  if (segments.value[edit.value]?.kept === false) {
    state.value = 'unavailable'
    sides.value = null
    return
  }
  const current = new AbortController()
  controller = current
  state.value = 'loading'
  failure.value = null
  try {
    const result = await source.value.read(path, current.signal, edit.value)
    if (current.signal.aborted) {
      return
    }
    sides.value = result
    state.value = result === null ? 'unavailable' : 'ready'
  } catch (error) {
    if (current.signal.aborted) {
      return
    }
    state.value = 'failed'
    failure.value = error instanceof Error ? error.message : String(error)
  }
}

watch(() => [source.value, selected.value, edit.value], read, { immediate: true })

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
          <span class="truncate" :class="selectedChange.kind === 'deleted' ? 'line-through' : ''" :title="selectedChange.path">{{ displayPath }}</span>
          <span class="shrink-0 tabular-nums">
            <span v-if="selectedChange.added > 0" class="text-on-success">+{{ selectedChange.added }}</span>
            <span v-if="selectedChange.removed > 0" class="ml-1 text-on-danger">−{{ selectedChange.removed }}</span>
          </span>
        </span>
        <div v-if="segments.length > 1" class="flex shrink-0 items-center gap-1 text-xs text-fg-muted">
          <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Previous edit" :disabled="edit === 0" @click="edit -= 1" />
          <span class="whitespace-nowrap">Edit {{ edit + 1 }} of {{ segments.length }}</span>
          <IconButton :icon="ArrowRight" variant="ghost" aria-label="Next edit" :disabled="edit >= segments.length - 1" @click="edit += 1" />
        </div>
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
    </div>
    <div class="flex min-h-0 flex-1 border-t border-line">
      <div class="relative min-w-0 flex-1">
        <!-- A diff is built for one pair of texts: a new file is a new editor. -->
        <DiffEditor
          v-if="state === 'ready' && sides && selectedChange"
          :key="`${source.identity ?? mode}:${selectedChange.path}:${edit}`"
          :host="appEditorHost"
          :original="sides.original"
          :modified="sides.modified"
          :filename="selectedChange.path"
          :resource-uri="toEditorUri('workspace', changeAbsolutePath(selectedChange.path, root))"
          :collapse-unchanged="collapseUnchanged"
        />
        <RegionStatus
          v-else-if="state !== 'unavailable'"
          class="h-full"
          :busy="state === 'loading'"
          :failed="state === 'failed'"
          :label="state === 'loading' ? 'Reading…' : state === 'failed' ? 'Could not read this change.' : idleText"
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
          :source="source"
          :root="root"
          :root-name="rootName"
          :selected="selected"
          :empty-text="emptyText"
          @select="selected = $event"
        />
      </template>
    </div>
  </div>
</template>

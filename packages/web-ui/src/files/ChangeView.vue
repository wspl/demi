<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, Diff, Eye, FileOutput } from '@lucide/vue'
import DiffEditor from '../editor/components/DiffEditor.vue'
import type { DocumentPlace } from '../markdown/document'
import IconButton from '../ui/IconButton.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tooltip from '../ui/Tooltip.vue'
import ChangeTree from './ChangeTree.vue'
import FilePreview from './FilePreview.vue'
import FileSummary from './FileSummary.vue'
import ImagePreview from './ImagePreview.vue'
import MarkdownDocument from './MarkdownDocument.vue'
import PreviewPair from './PreviewPair.vue'
import TreeFrame from './TreeFrame.vue'
import { emptyChangeSetText, type ChangeMode, type ChangeSides, type ChangeSources } from './changes'
import { TREE_WIDTH } from './file-view'
import { baseName, resolveHostPath } from './paths'
import { hasSourceView, previewKind, svgImageUrl } from './preview'
import { FileBrowserError, type FileContents } from './types'

/**
 * One working-tree file or one retained segment of a call. The working-tree
 * sidebar, segment control and Back/Forward events use the selection held by the host.
 * A missing retained pair leaves the editor area empty. The sidebar docks,
 * hides and shows over the diff as its frame (`TreeFrame`) decides.
 *
 * Text shows as a diff. In Uncommitted mode an image, a video, an audio file
 * or a PDF shows its committed and working-tree versions side by side, and a
 * binary file without a preview a card per side (`file-previews.md`
 * § Changes). Markdown and SVG offer Diff and Preview, which renders both
 * sides; the host keeps the choice.
 */
const props = defineProps<{
  changes: ChangeSources
  /** The workspace the paths are relative to. */
  root: string
  /** What the tree's caption says in place of the root directory's name. */
  rootName?: string
  /** The working tree's bytes, by absolute path; previews need them. */
  contents?: FileContents
  canBack?: boolean
  canForward?: boolean
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
/** Markdown and SVG only: the text diff, or both sides rendered. */
const presentation = defineModel<'diff' | 'preview'>('presentation', { default: 'diff' })

const modeOptions: readonly SegmentedOption<ChangeMode>[] = [
  { value: 'uncommitted', label: 'Uncommitted' },
  { value: 'conversation', label: 'Conversation' },
]
const presentationOptions: readonly SegmentedOption<'diff' | 'preview'>[] = [
  { value: 'diff', label: 'Diff', icon: Diff },
  { value: 'preview', label: 'Preview', icon: Eye },
]
/** Git's copy is decoded whole, so the committed side stops at the runner's limit (`file-previews.md` § Changes). */
const COMMITTED_TOO_LARGE = 'Over 8 MiB: too large to show.'

const workingTree = computed(() => props.changes.uncommitted)
const call = computed(() => mode.value === 'conversation' ? props.changes.conversation : null)
const selectedChange = computed(() => mode.value === 'conversation'
  ? call.value?.file ?? null
  : workingTree.value.files.find((file) => file.path === selected.value) ?? null)
const segments = computed(() => call.value?.file.edits ?? [])
const absolutePath = computed(() => resolveHostPath(props.root, selectedChange.value?.path ?? ''))
const treeAvailable = computed(() => mode.value === 'uncommitted' && workingTree.value.unavailable !== 'no-repository')
const emptyText = computed(() => emptyChangeSetText(workingTree.value))
const idleText = computed(() => mode.value === 'conversation'
  ? 'Click a changed file in the conversation to see its diff here.'
  : workingTree.value.files.length > 0 ? 'Select a changed file.' : emptyText.value)

const kind = computed(() => selectedChange.value ? previewKind(selectedChange.value.path) : 'text')
const sourceView = computed(() => selectedChange.value !== null && hasSourceView(selectedChange.value.path))
/** The committed side's path: a renamed file's is its old one. */
const committedPath = computed(() => {
  const change = selectedChange.value
  if (!change)
    return ''
  return 'from' in change && change.from ? change.from : change.path
})
/** In Uncommitted mode, media compares its two versions from their bytes, not their text. */
const mediaPair = computed(() => {
  if (mode.value !== 'uncommitted' || !props.contents || !workingTree.value.committed)
    return null
  const shown = kind.value
  // SVG is text as well: it diffs, and Preview renders both sides.
  if (shown === 'text' || shown === 'markdown' || sourceView.value)
    return null
  return shown
})
/** Whether the change has a version before it: not a file this change created. */
const hasBefore = computed(() => {
  const change = selectedChange.value
  if (!change)
    return false
  return mode.value === 'conversation' ? !(change.kind === 'added' && edit.value === 0) : change.kind !== 'added'
})
const hasAfter = computed(() => selectedChange.value?.kind !== 'deleted')
const labels = computed(() => mode.value === 'conversation'
  ? { before: 'Before', after: 'After' }
  : { before: 'Committed', after: 'Working tree' })
const place = computed<DocumentPlace>(() => ({
  path: absolutePath.value,
  root: props.root,
  // Relative images load as the Host has them now.
  imageUrl: (path) => props.contents?.url(path) ?? '',
}))

type State =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'ready'; sides: ChangeSides }
  | { phase: 'media' }
  | { phase: 'binary' }
  | { phase: 'failed'; message: string }
  | { phase: 'unavailable' }

const state = ref<State>({ phase: 'idle' })
let controller: AbortController | null = null

async function read(): Promise<void> {
  controller?.abort()
  controller = null
  const path = selectedChange.value?.path
  if (path === undefined) {
    state.value = { phase: 'idle' }
    return
  }
  if (segments.value[edit.value]?.kept === false) {
    state.value = { phase: 'unavailable' }
    return
  }
  if (mediaPair.value) {
    state.value = { phase: 'media' }
    return
  }
  const current = new AbortController()
  controller = current
  state.value = { phase: 'loading' }
  try {
    const result = call.value
      ? await call.value.read(edit.value, current.signal)
      : await workingTree.value.read(path, current.signal)
    if (current.signal.aborted)
      return
    state.value = result === null ? { phase: 'unavailable' } : { phase: 'ready', sides: result }
  } catch (error) {
    if (current.signal.aborted)
      return
    // A side that is not text: each version as a card of its own.
    if (error instanceof FileBrowserError && (error.kind === 'binary' || error.kind === 'too-large')) {
      state.value = { phase: 'binary' }
      return
    }
    state.value = { phase: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}

watch(
  () => [mode.value === 'conversation' ? call.value : workingTree.value, selectedChange.value, edit.value, mediaPair.value],
  read,
  { immediate: true },
)

const frame = ref<InstanceType<typeof TreeFrame> | null>(null)

function pick(path: string): void {
  frame.value?.dismiss()
  selected.value = path
}

onBeforeUnmount(() => {
  controller?.abort()
})
</script>

<template>
  <TreeFrame ref="frame" v-model:open="tree" v-model:width="treeWidth" name="changed files">
    <template #header>
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
      <!-- The file's controls keep to the right, whether or not a file is shown. -->
      <div class="ml-auto flex items-center gap-1">
        <div v-if="segments.length > 1" class="flex shrink-0 items-center gap-1 text-xs text-fg-muted">
          <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Previous edit" :disabled="edit === 0" @click="edit -= 1" />
          <span class="whitespace-nowrap">Edit {{ edit + 1 }} of {{ segments.length }}</span>
          <IconButton :icon="ArrowRight" variant="ghost" aria-label="Next edit" :disabled="edit >= segments.length - 1" @click="edit += 1" />
        </div>
        <Segmented
          v-if="sourceView && selectedChange"
          v-model="presentation"
          :options="presentationOptions"
          size="sm"
          icon-only
          class="shrink-0"
        />
        <Tooltip content="Open file" class="shrink-0">
          <IconButton
            :icon="FileOutput"
            variant="ghost"
            aria-label="Open file"
            :disabled="!selectedChange || selectedChange.kind === 'deleted'"
            @click="selectedChange && emit('open', selectedChange.path)"
          />
        </Tooltip>
      </div>
    </template>
    <PreviewPair
      v-if="state.phase === 'media' && mediaPair && contents && changes.uncommitted.committed"
      :key="`${committedPath}:${absolutePath}`"
      v-bind="labels"
    >
      <template v-if="hasBefore" #before>
        <FilePreview
          :path="committedPath"
          :kind="mediaPair"
          :contents="changes.uncommitted.committed"
          :too-large="COMMITTED_TOO_LARGE"
        />
      </template>
      <template v-if="hasAfter" #after>
        <FilePreview :path="absolutePath" :kind="mediaPair" :contents="contents" />
      </template>
    </PreviewPair>
    <PreviewPair v-else-if="state.phase === 'binary'" :key="`${committedPath}:${absolutePath}`" v-bind="labels">
      <template v-if="hasBefore" #before>
        <FileSummary :path="committedPath" :contents="changes.uncommitted.committed" :too-large="COMMITTED_TOO_LARGE" />
      </template>
      <template v-if="hasAfter" #after>
        <FileSummary :path="absolutePath" :contents="contents" />
      </template>
    </PreviewPair>
    <PreviewPair
      v-else-if="state.phase === 'ready' && sourceView && presentation === 'preview'"
      :key="`${call?.commandId ?? mode}:${absolutePath}:${edit}`"
      v-bind="labels"
    >
      <template v-if="hasBefore" #before>
        <MarkdownDocument v-if="kind === 'markdown'" :text="state.sides.original" :place="place" @open="emit('open', $event)" />
        <ImagePreview v-else :src="svgImageUrl(state.sides.original)" :name="baseName(committedPath)" />
      </template>
      <template v-if="hasAfter" #after>
        <MarkdownDocument v-if="kind === 'markdown'" :text="state.sides.modified" :place="place" @open="emit('open', $event)" />
        <ImagePreview v-else :src="svgImageUrl(state.sides.modified)" :name="baseName(absolutePath)" />
      </template>
    </PreviewPair>
    <!-- A diff is built for one pair of texts: a new file is a new editor. -->
    <DiffEditor
      v-else-if="state.phase === 'ready' && selectedChange"
      :key="`${call?.commandId ?? mode}:${selectedChange.path}:${edit}`"
      :original="state.sides.original"
      :modified="state.sides.modified"
      :path="selectedChange.path"
    />
    <RegionStatus
      v-else-if="state.phase !== 'unavailable'"
      class="h-full"
      :busy="state.phase === 'loading'"
      :failed="state.phase === 'failed'"
      :label="state.phase === 'loading' ? 'Reading…' : state.phase === 'failed' ? 'Could not read this change.' : idleText"
      :detail="state.phase === 'failed' ? state.message : null"
      :action="state.phase === 'failed' ? 'Retry' : undefined"
      @action="read"
    />
    <template v-if="treeAvailable" #tree>
      <ChangeTree
        :source="workingTree"
        :root="root"
        :root-name="rootName"
        :selected="selected"
        :empty-text="emptyText"
        @select="pick"
      />
    </template>
  </TreeFrame>
</template>

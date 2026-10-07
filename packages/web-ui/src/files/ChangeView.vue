<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, Diff, Eye, FileOutput, History, RefreshCw } from '@lucide/vue'
import DiffEditor from '../editor/components/DiffEditor.vue'
import type { DocumentPlace } from '../markdown/document'
import IconButton from '../ui/IconButton.vue'
import RegionNote from '../ui/RegionNote.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tooltip from '../ui/Tooltip.vue'
import ChangeTree from './ChangeTree.vue'
import { useShowing } from './showing'
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
  /** Whether the page can open a file, so the view offers Open file and follows links to files. */
  opens?: boolean
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

// A working-tree file's sides, as the change set keeps them: a file shown a
// moment ago shows at once, and sides read again replace the diff in place.
const workingSides = useShowing(
  () => mode.value === 'uncommitted' && !mediaPair.value ? workingTree.value : null,
  () => selectedChange.value?.path,
  (source, path) => source.showSides(path),
)

/** The working-tree side's state, as the kept sides say. */
const workingState = computed<State>(() => {
  if (!selectedChange.value)
    return { phase: 'idle' }
  if (mediaPair.value)
    return { phase: 'media' }
  const entry = workingSides.entry.value
  if (!entry)
    return { phase: 'loading' }
  if (entry.value !== undefined)
    return { phase: 'ready', sides: entry.value }
  const failure = entry.failure
  if (!failure)
    return { phase: 'loading' }
  // A side that is not text: each version as a card of its own.
  if (failure.kind === 'binary' || failure.kind === 'too-large')
    return { phase: 'binary' }
  return { phase: 'failed', message: failure.message ?? 'The change could not be read.' }
})

/** Why the last read of the sides shown failed, while the sides it had stay. */
const staleBecause = computed(() => {
  const entry = mode.value === 'uncommitted' ? workingSides.entry.value : null
  return entry?.value !== undefined && entry.failure ? entry.failure.message ?? 'The read failed.' : null
})

// A call's retained edit, read from its copies, which never change.
const callState = ref<State>({ phase: 'idle' })
let controller: AbortController | null = null

async function readCall(): Promise<void> {
  controller?.abort()
  controller = null
  const shown = call.value
  if (!shown || !selectedChange.value) {
    callState.value = { phase: 'idle' }
    return
  }
  // An edit without copies has no diff to show.
  if (!segments.value[edit.value]?.copies) {
    callState.value = { phase: 'unavailable' }
    return
  }
  const current = new AbortController()
  controller = current
  callState.value = { phase: 'loading' }
  try {
    const result = await shown.read(edit.value, current.signal)
    if (current.signal.aborted)
      return
    callState.value = result === null ? { phase: 'unavailable' } : { phase: 'ready', sides: result }
  } catch (error) {
    if (current.signal.aborted)
      return
    if (error instanceof FileBrowserError && (error.kind === 'binary' || error.kind === 'too-large')) {
      callState.value = { phase: 'binary' }
      return
    }
    callState.value = { phase: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}

watch(() => [call.value, edit.value], readCall, { immediate: true })

const state = computed<State>(() => mode.value === 'conversation' ? callState.value : workingState.value)

function retry(): void {
  if (mode.value === 'conversation')
    void readCall()
  else
    workingSides.retry()
}

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
        <Tooltip v-if="opens !== false" content="Open file" class="shrink-0">
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
    <div class="flex h-full min-h-0 flex-col">
      <RegionNote
        v-if="mode === 'uncommitted' && workingTree.watch?.unavailable"
        :icon="History"
        label="Showing files as they were last read."
        :detail="workingTree.watch.unavailable"
        action="Refresh"
        @action="workingTree.watch.refresh()"
      />
      <RegionNote
        v-if="staleBecause"
        :icon="RefreshCw"
        label="Could not refresh this change."
        :detail="staleBecause"
        action="Retry"
        @action="retry"
      />
      <div class="min-h-0 flex-1">
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
          <MarkdownDocument v-if="kind === 'markdown'" :text="state.sides.original" :place="place" @open="opens !== false && emit('open', $event)" />
          <ImagePreview v-else :src="svgImageUrl(state.sides.original)" :name="baseName(committedPath)" />
        </template>
        <template v-if="hasAfter" #after>
          <MarkdownDocument v-if="kind === 'markdown'" :text="state.sides.modified" :place="place" @open="opens !== false && emit('open', $event)" />
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
        :status="state.phase === 'loading' || state.phase === 'failed' ? state.phase : 'note'"
        :label="state.phase === 'loading' ? 'Reading…' : state.phase === 'failed' ? 'Could not read this change.' : idleText"
        loading-label="Reading…"
        :detail="state.phase === 'failed' ? state.message : null"
        :on-retry="retry"
      />
      </div>
    </div>
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

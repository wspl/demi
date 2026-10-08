<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, ChevronLeft, ChevronRight, Diff, Eye, FileOutput, History, RefreshCw } from '@lucide/vue'
import DiffEditor from '../editor/components/DiffEditor.vue'
import type { DocumentPlace } from '../markdown/document'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import RegionNote from '../ui/RegionNote.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tooltip from '../ui/Tooltip.vue'
import ChangeTree from './ChangeTree.vue'
import RequestFileList from './RequestFileList.vue'
import { useShowing } from './showing'
import FilePreview from './FilePreview.vue'
import FileIcon from './FileIcon.vue'
import FileSummary from './FileSummary.vue'
import LineCounts from './LineCounts.vue'
import ImagePreview from './ImagePreview.vue'
import MarkdownDocument from './MarkdownDocument.vue'
import PreviewPair from './PreviewPair.vue'
import TreeFrame from './TreeFrame.vue'
import { emptyChangeSetText, type ChangeMode, type ChangeSides, type ChangeSources } from './changes'
import { editIndex, offersAllChanges, selectionCopies, type RequestEditRef } from './request-changes'
import { TREE_WIDTH } from './file-view'
import { baseName } from '@demicodes/utils'
import { relativePath, resolveHostPath } from './paths'
import { hasSourceView, previewKind, svgImageUrl } from './preview'
import { FileBrowserError, type FileContents } from './types'

/**
 * One working-tree file, or one file of a request of the conversation
 * (`edit-tracking.md` § What the conversation shows): its All Changes, from
 * before the request's first edit of it to after its last, or one of its
 * edits, which a control steps through. The sidebar lists the working
 * tree's files, or the request's; the file, the edit and Back and Forward
 * are the host's to hold. An edit whose two sides were not kept leaves the
 * editor area empty. The sidebar docks, hides and shows over the diff as its
 * frame (`TreeFrame`) decides.
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
/** In Conversation, the selected file's edit shown; null for All Changes. */
const edit = defineModel<RequestEditRef | null>('edit', { default: null })
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
const request = computed(() => mode.value === 'conversation' ? props.changes.conversation : null)
const requestFile = computed(() => request.value?.files.find((file) => file.path === selected.value) ?? null)
const selectedChange = computed(() => mode.value === 'conversation'
  ? requestFile.value
  : workingTree.value.files.find((file) => file.path === selected.value) ?? null)
const edits = computed(() => requestFile.value?.edits ?? [])
/** Whether the file offers All Changes: both its ends were kept. */
const allChanges = computed(() => requestFile.value !== null && offersAllChanges(requestFile.value))
/** Where the shown edit stands among the file's edits; null for All Changes. */
const editAt = computed(() => requestFile.value ? editIndex(requestFile.value, edit.value) : null)
/** The two sides the selection shows, as the blobs that hold them; null when one was not kept. */
const requestCopies = computed(() => requestFile.value ? selectionCopies(requestFile.value, editAt.value) : null)
/** Names the pair shown, which a new pair replaces: the blobs, whatever object holds them. */
const requestKey = computed(() => requestCopies.value ? `${requestCopies.value.original}:${requestCopies.value.modified}` : null)
/** The step the edit control goes to, each way: All Changes, where offered, comes before the first edit. */
const previousEdit = computed(() => {
  const at = editAt.value
  if (at === null)
    return undefined
  return at > 0 ? edits.value[at - 1] : allChanges.value ? null : undefined
})
const nextEdit = computed(() => {
  const at = editAt.value
  return at === null ? edits.value[0] : edits.value[at + 1]
})
const editLabel = computed(() => editAt.value === null ? 'All Changes' : `Edit ${editAt.value + 1} of ${edits.value.length}`)
const editItems = computed(() => [
  ...(allChanges.value ? [{ id: 'all', label: 'All Changes' }] : []),
  ...edits.value.map((entry, index) => ({ id: String(index), label: entry.title, value: `Edit ${index + 1}` })),
])

function showEdit(target: { call: string; segment: number } | null | undefined): void {
  if (target !== undefined) {
    edit.value = target === null ? null : { call: target.call, segment: target.segment }
  }
}

function chooseEdit(id: string, close: () => void): void {
  showEdit(id === 'all' ? null : edits.value[Number(id)])
  close()
}
const absolutePath = computed(() => resolveHostPath(props.root, selectedChange.value?.path ?? ''))
/**
 * The file the view shows, named over it as a diff's file header names it on
 * GitHub: its folder from the workspace, its name, and where a renamed file
 * came from.
 */
const heading = computed(() => {
  const change = selectedChange.value
  if (!change)
    return null
  const shown = relativePath(props.root, absolutePath.value)
  const folder = shown.slice(0, shown.lastIndexOf('/') + 1)
  const from = 'from' in change && change.from ? relativePath(props.root, resolveHostPath(props.root, change.from)) : null
  return { folder, name: baseName(absolutePath.value), from, deleted: change.kind === 'deleted' }
})
const treeAvailable = computed(() => mode.value === 'uncommitted'
  ? workingTree.value.unavailable !== 'no-repository'
  : request.value !== null)
const emptyText = computed(() => emptyChangeSetText(workingTree.value))
const idleText = computed(() => {
  if (mode.value === 'conversation') {
    if (!request.value)
      return 'Click a changed file in the conversation to see its diff here.'
    return request.value.files.length > 0 ? 'Select a changed file.' : 'These changes are no longer in the conversation.'
  }
  return workingTree.value.files.length > 0 ? 'Select a changed file.' : emptyText.value
})

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
  if (mode.value === 'uncommitted')
    return change.kind !== 'added'
  // A request's edit that created the file, or All Changes from before it, has nothing before it.
  return !edits.value[editAt.value ?? 0]?.created
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

// A request's file, read from the copies the selection names, which never change.
const callState = ref<State>({ phase: 'idle' })
let controller: AbortController | null = null

async function readCall(): Promise<void> {
  controller?.abort()
  controller = null
  const shown = request.value
  if (!shown || !requestFile.value) {
    callState.value = { phase: 'idle' }
    return
  }
  // A selection without both sides kept has no diff to show.
  const copies = requestCopies.value
  if (!copies) {
    callState.value = { phase: 'unavailable' }
    return
  }
  const current = new AbortController()
  controller = current
  callState.value = { phase: 'loading' }
  try {
    const result = await shown.read(copies, current.signal)
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

// The transcript is derived anew as it grows: only another pair of sides, or another file, is read again.
watch(() => [request.value !== null, requestFile.value?.path, requestKey.value], readCall, { immediate: true })

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
        <!-- All Changes, then each edit of the request in turn, as GitHub picks a pull request's commits. -->
        <div v-if="mode === 'conversation' && edits.length > 1" class="flex shrink-0 items-center">
          <Tooltip content="Previous edit">
            <IconButton :icon="ChevronLeft" variant="ghost" aria-label="Previous edit" :disabled="previousEdit === undefined" @click="showEdit(previousEdit)" />
          </Tooltip>
          <Dropdown :overlay-store="appOverlayStore" variant="ghost" size="sm" placement="bottom-end">
            <template #trigger>
              <span class="whitespace-nowrap text-xs">{{ editLabel }}</span>
            </template>
            <template #content="{ close }">
              <Menu
                class="w-80"
                :items="editItems"
                :selected-id="editAt === null ? 'all' : String(editAt)"
                :item-height="28"
                @select="chooseEdit($event, close)"
              />
            </template>
          </Dropdown>
          <Tooltip content="Next edit">
            <IconButton :icon="ChevronRight" variant="ghost" aria-label="Next edit" :disabled="nextEdit === undefined" @click="showEdit(nextEdit)" />
          </Tooltip>
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
    <template #default="{ overlap }">
    <div class="flex h-full min-h-0 flex-col">
      <div
        v-if="heading && selectedChange"
        class="flex h-8 shrink-0 select-none items-center gap-1.5 border-b border-line px-3 text-[12px]"
      >
        <FileIcon :name="heading.name" :is-directory="false" :size="14" class="shrink-0" />
        <span v-if="heading.from" class="min-w-0 truncate text-fg-muted">{{ heading.from }} →</span>
        <!-- Path and counts use different fonts and sizes: they align on the baseline, as the file pills do. -->
        <span class="flex min-w-0 items-baseline gap-2.5">
          <!-- One path: its folders give way from the left, and its name stays whole (the gallery's Writing page). -->
          <span class="flex min-w-0 items-baseline">
            <span class="min-w-0 truncate text-fg-muted [direction:rtl]"><bdi>{{ heading.folder }}</bdi></span>
            <span class="shrink-0" :class="heading.deleted ? 'text-fg-muted line-through' : 'text-fg'">{{ heading.name }}</span>
          </span>
          <LineCounts :added="selectedChange.added" :removed="selectedChange.removed" />
        </span>
        <!-- The edit shown is named by its call's title; the file's name keeps its width first. -->
        <span v-if="editAt !== null" class="ml-auto min-w-0 truncate pl-2 text-fg-subtle" :title="edits[editAt]?.title">{{ edits[editAt]?.title }}</span>
      </div>
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
        :key="`${mode}:${absolutePath}:${requestKey}`"
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
        :key="`${mode}:${selectedChange.path}:${requestKey}`"
        :original="state.sides.original"
        :modified="state.sides.modified"
        :path="selectedChange.path"
      />
      <!-- It centres in the space beside a tree shown over the view. -->
      <div v-else-if="state.phase !== 'unavailable'" class="h-full" :style="{ paddingInlineEnd: `${overlap}px` }">
        <RegionStatus
          class="h-full"
          :status="state.phase === 'loading' || state.phase === 'failed' ? state.phase : 'note'"
          :label="state.phase === 'loading' ? 'Reading…' : state.phase === 'failed' ? 'Could not read this change.' : idleText"
          loading-label="Reading…"
          :detail="state.phase === 'failed' ? state.message : null"
          :on-retry="retry"
        />
      </div>
      </div>
    </div>
    </template>
    <template v-if="treeAvailable" #tree>
      <ChangeTree
        v-if="mode === 'uncommitted'"
        :source="workingTree"
        :root="root"
        :root-name="rootName"
        :selected="selected"
        :empty-text="emptyText"
        @select="pick"
      />
      <RequestFileList
        v-else-if="request"
        :files="request.files"
        :root="root"
        :root-name="rootName"
        :selected="selected"
        @select="pick"
        @uncommitted="mode = 'uncommitted'"
      />
    </template>
  </TreeFrame>
</template>

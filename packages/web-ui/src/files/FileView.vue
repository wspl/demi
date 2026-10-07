<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, Code, Download, Eye, FileText, History, RefreshCw } from '@lucide/vue'
import type { StateEffect } from '@codemirror/state'
import CodeEditor from '../editor/components/CodeEditor.vue'
import { showToast } from '../infra/toast'
import { renderable, type DocumentPlace } from '../markdown/document'
import IconButton from '../ui/IconButton.vue'
import RegionNote from '../ui/RegionNote.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tooltip from '../ui/Tooltip.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FilePreview from './FilePreview.vue'
import FileSummary from './FileSummary.vue'
import FileTree from './FileTree.vue'
import MarkdownDocument from './MarkdownDocument.vue'
import TreeFrame from './TreeFrame.vue'
import { downloadUrl } from './download'
import { TREE_WIDTH } from './file-view'
import { formatBytes } from './format'
import { baseName } from '@demicodes/utils'
import { normalizePath, parentPath } from './paths'
import { hasSourceView, previewKind, TOO_LARGE_NOTE } from './preview'
import { useShowing } from './showing'
import { useTextStart } from './text-start'
import type { FileBrowserSource } from './types'

/**
 * One file of a workspace, read through its source: the path as crumbs from
 * the workspace root, the file itself, and beside it the workspace tree with
 * the file selected. Text opens read-only in the code editor. An image, a
 * video, an audio file or a PDF shows in the web browser's own viewer,
 * Markdown renders as a document, and a file the page cannot show is a card
 * (`file-previews.md`). Markdown and SVG offer Preview and Source, and every
 * file can be downloaded.
 *
 * The tree docks, hides and shows over the file as its frame (`TreeFrame`)
 * decides; the host keeps whether it is open, its width, and the Preview or
 * Source choice (v-model), so they hold across files. A text file too large
 * to read whole shows its start, with a line saying how much of it shows and
 * Download (`file-previews.md` § Getting the bytes). A click on another file
 * in the tree, a pick from a crumb's menu, or a link in a Markdown document
 * asks the host to show it here; Back and Forward before the crumbs ask for
 * the files shown before and after. A path typed into the crumb row opens
 * that file, or finds that folder in the tree and selects it there. Files
 * dropped on the tree, or uploaded from its menu, list under it until
 * cleared (`FileTree`).
 *
 * The file shows as its source keeps it (`plugin-pages.md` § What the
 * service keeps): a file seen a moment ago shows at once, scrolled where it
 * was, and a new text replaces the old in place. A read that fails keeps
 * what shows and says so quietly, with Retry; a Host that cannot watch its
 * files has the view say that it shows them as last read, with Refresh.
 */
const props = defineProps<{
  source: FileBrowserSource
  /** The workspace: the tree's root and the first crumb. */
  root: string
  /** What the first crumb and the tree's caption say in place of the root directory's name. */
  rootName?: string
  /** The file, by absolute path. */
  path: string | null
  /** Whether the host has a file to go back or forward to in this view. */
  canBack?: boolean
  canForward?: boolean
}>()

const emit = defineEmits<{
  open: [path: string]
  back: []
  forward: []
}>()

const tree = defineModel<boolean>('tree', { default: true })

const treeWidth = defineModel<number>('treeWidth', { default: TREE_WIDTH.default })

/** Markdown and SVG only: their rendered view, or their text. */
const mode = defineModel<'preview' | 'source'>('mode', { default: 'preview' })

const modeOptions: readonly SegmentedOption<'preview' | 'source'>[] = [
  { value: 'preview', label: 'Preview', icon: Eye },
  { value: 'source', label: 'Source', icon: Code },
]

const kind = computed(() => props.path === null ? 'text' : previewKind(props.path))
const sourceView = computed(() => props.path !== null && hasSourceView(props.path))
/** An image, video, audio file or PDF shown from its bytes, SVG only while Preview is chosen. */
const media = computed(() => {
  const shown = kind.value
  if (shown === 'text' || shown === 'markdown' || !props.source.contents)
    return null
  return sourceView.value && mode.value === 'source' ? null : shown
})

type TextState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'ready'; text: string }
  /** A file the text read cannot show; a text file too large for it shows its start instead. */
  | { phase: 'card'; tooLarge: boolean }
  | { phase: 'failed'; message: string }

const shown = useShowing(
  () => props.source,
  () => props.path !== null && media.value === null ? props.path : null,
  (source, path) => source.showText?.(path),
)

const state = computed<TextState>(() => {
  if (props.path === null || media.value !== null)
    return { phase: 'idle' }
  if (!props.source.showText)
    return { phase: 'failed', message: 'This source cannot read files.' }
  const entry = shown.entry.value
  if (!entry)
    return { phase: 'loading' }
  if (entry.value !== undefined)
    return { phase: 'ready', text: entry.value.text }
  const failure = entry.failure
  if (!failure)
    return { phase: 'loading' }
  if (failure.kind === 'binary' || failure.kind === 'too-large')
    return { phase: 'card', tooLarge: failure.kind === 'too-large' }
  return { phase: 'failed', message: failure.message ?? 'The file could not be read.' }
})

/** How many times the text shown was read again successfully: a document's images that did not load try again then. */
const reloads = ref(0)
watch(() => shown.entry.value?.reading, (reading, wasReading) => {
  if (wasReading && !reading && !shown.entry.value?.failure)
    reloads.value += 1
})

/** Why the last read of the text shown failed, while the text it had stays. */
const staleBecause = computed(() => {
  const entry = shown.entry.value
  return entry?.value !== undefined ? entry.failure?.message ?? (entry.failure ? 'The read failed.' : null) : null
})

/** The start of a text file too large for the text read, while one is shown. */
const start = useTextStart(
  () => props.source.contents,
  () => state.value.phase === 'card' && state.value.tooLarge ? props.path : null,
)
/** The line over a large file's start: how much of how much shows. */
const startNote = computed(() => start.value?.phase === 'ready'
  ? `Showing the first ${formatBytes(start.value.shown)} of ${formatBytes(start.value.size)}.`
  : null)

/** What the region says while it shows no file: none chosen, a read on its way, or why the read failed. */
const placeholder = computed(() => {
  const current = state.value
  if (current.phase === 'failed')
    return { status: 'failed' as const, label: 'Could not read this file.', detail: current.message }
  if (current.phase === 'idle')
    return { status: 'note' as const, label: 'Select a file.', detail: null }
  return { status: 'loading' as const, label: 'Reading…', detail: null }
})

/** Where each file this view showed was scrolled to, to open there again. */
const scrolls = new Map<string, StateEffect<unknown>>()

/** Rendered Markdown, unless Source is chosen or the document is too large to render. */
const markdown = computed(() => kind.value === 'markdown' &&
  mode.value === 'preview' &&
  state.value.phase === 'ready' &&
  renderable(state.value.text))
const tooLargeToRender = computed(() => kind.value === 'markdown' &&
  (state.value.phase === 'ready' ? !renderable(state.value.text) : start.value?.phase === 'ready'))
const place = computed<DocumentPlace | null>(() => props.path === null
  ? null
  : {
      path: props.path,
      root: props.root,
      imageUrl: (path) => props.source.contents?.url(path) ?? '',
    })

// A directory typed into the crumb row: selected in the tree until another file opens.
const located = ref<string | null>(null)
const frame = ref<InstanceType<typeof TreeFrame> | null>(null)
const treeView = ref<InstanceType<typeof FileTree> | null>(null)
let going: AbortController | null = null

watch(() => props.path, () => {
  located.value = null
})

/** Whether `path` names a directory, by its parent's listing. */
async function isDirectory(path: string): Promise<boolean> {
  // The filesystem root has no parent to list.
  if (path === '/')
    return true
  try {
    const entries = await props.source.list(parentPath(path))
    return entries.some((entry) => entry.name === baseName(path) && entry.isDirectory)
  } catch {
    // Opened as a file instead, whose read says what is wrong.
    return false
  }
}

/**
 * Where a path typed into the crumb row goes: a directory is found in the
 * tree, unfolded to and selected; anything else opens as a file, and one that
 * is not there says so the way any failed read does.
 */
async function go(target: string): Promise<void> {
  going?.abort()
  const current = new AbortController()
  going = current
  const directory = await isDirectory(target)
  // Another path was typed meanwhile, or the view went away.
  if (current.signal.aborted)
    return
  if (!directory) {
    emit('open', target)
    return
  }
  const root = normalizePath(props.root)
  if (target !== root && !target.startsWith(`${root}/`)) {
    showToast({ title: 'Not in This Workspace', message: `The tree shows ${props.rootName ?? root} only.`, tone: 'neutral' })
    return
  }
  located.value = target
  frame.value?.show()
  await nextTick()
  treeView.value?.reveal(target)
}

function openFromTree(path: string): void {
  frame.value?.dismiss()
  emit('open', path)
}

function download(): void {
  if (props.path !== null && props.source.contents)
    downloadUrl(props.source.contents.url(props.path, { download: true }))
}

onBeforeUnmount(() => {
  going?.abort()
})
</script>

<template>
  <TreeFrame ref="frame" v-model:open="tree" v-model:width="treeWidth" name="file tree">
    <template #header>
      <!-- Back and Forward move through the files this view has shown. -->
      <div class="flex shrink-0 items-center">
        <Tooltip content="Back">
          <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Back" :disabled="!canBack" @click="emit('back')" />
        </Tooltip>
        <Tooltip content="Forward">
          <IconButton :icon="ArrowRight" variant="ghost" aria-label="Forward" :disabled="!canForward" @click="emit('forward')" />
        </Tooltip>
      </div>
      <!-- Crumbs open menus of what lies beside them: another file is a pick away. -->
      <FileBrowserAddressBar
        class="ml-2 min-w-0 flex-1"
        mode="browse"
        :path="path ?? root"
        :root="root"
        :root-name="rootName"
        :source="source"
        :leaf="path ? 'file' : 'directory'"
        @navigate="go"
        @open="emit('open', $event)"
      />
      <Segmented
        v-if="sourceView"
        v-model="mode"
        :options="modeOptions"
        size="sm"
        icon-only
        class="shrink-0"
        :disabled="tooLargeToRender"
        disabled-reason="Too large to render; showing its source"
      />
      <Tooltip v-if="source.contents" content="Download" class="shrink-0">
        <IconButton :icon="Download" variant="ghost" aria-label="Download" :disabled="path === null" @click="download" />
      </Tooltip>
    </template>
    <div class="flex h-full min-h-0 flex-col">
      <RegionNote
        v-if="source.watch?.unavailable"
        :icon="History"
        label="Showing files as they were last read."
        :detail="source.watch.unavailable"
        action="Refresh"
        @action="source.watch.refresh()"
      />
      <RegionNote
        v-if="staleBecause"
        :icon="RefreshCw"
        label="Could not refresh this file."
        :detail="staleBecause"
        action="Retry"
        @action="shown.retry"
      />
      <RegionNote v-if="startNote" :icon="FileText" :label="startNote" action="Download" @action="download" />
      <div class="min-h-0 flex-1">
        <FilePreview
          v-if="media && path && source.contents"
          :key="path"
          :path="path"
          :kind="media"
          :contents="source.contents"
        />
        <MarkdownDocument
          v-else-if="markdown && state.phase === 'ready' && place"
          :text="state.text"
          :place="place"
          :reloads="reloads"
          @open="emit('open', $event)"
        />
        <!-- One editor per file: a new text of it replaces the old in place. -->
        <CodeEditor
          v-else-if="(state.phase === 'ready' || start?.phase === 'ready') && path"
          :key="path"
          class="h-full"
          :path="path"
          :text="state.phase === 'ready' ? state.text : start?.phase === 'ready' ? start.text : ''"
          :scroll-to="scrolls.get(path)"
          @left="(left, snapshot) => scrolls.set(left, snapshot)"
        />
        <FileSummary
          v-else-if="state.phase === 'card' && start?.phase !== 'loading' && path"
          :path="path"
          :contents="source.contents"
          :note="state.tooLarge ? TOO_LARGE_NOTE : null"
        />
        <RegionStatus
          v-else
          class="h-full"
          :status="placeholder.status"
          :label="placeholder.label"
          loading-label="Reading…"
          :detail="placeholder.detail"
          :on-retry="shown.retry"
        />
      </div>
    </div>
    <template #tree>
      <FileTree
        ref="treeView"
        :source="source"
        :root="root"
        :root-name="rootName"
        :selected="located ?? path"
        @open="openFromTree"
      />
    </template>
  </TreeFrame>
</template>

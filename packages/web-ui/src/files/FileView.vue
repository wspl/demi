<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, FileText, History, RefreshCw } from '@lucide/vue'
import type { StateEffect } from '@codemirror/state'
import CodeEditor from '../editor/components/CodeEditor.vue'
import { showToast } from '../infra/toast'
import { renderable, type DocumentPlace } from '../markdown/document'
import IconButton from '../ui/IconButton.vue'
import RegionNote from '../ui/RegionNote.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Tooltip from '../ui/Tooltip.vue'
import FileBar from './FileBar.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FilePreview from './FilePreview.vue'
import FileSummary from './FileSummary.vue'
import FileTree from './FileTree.vue'
import MarkdownDocument from './MarkdownDocument.vue'
import TreeFrame from './TreeFrame.vue'
import { downloadUrl } from './download'
import { fileBar, type ShownFile } from './file-bar'
import { TREE_WIDTH } from './file-view'
import { formatBytes, linkDescription } from './format'
import { baseName } from '@demicodes/utils'
import { normalizePath, parentPath } from './paths'
import { hasSourceView, MISSING_FILE, previewKind, TOO_LARGE_NOTE } from './preview'
import { useShowing } from './showing'
import { useTextStart } from './text-start'
import type { FileBrowserSource } from './types'

/**
 * One file of a Host, read through its source: the whole path as crumbs
 * from the Host's root, the file itself under a bar that says what it is
 * and holds what can be done with it, and beside it the workspace tree with
 * the file selected (`file-previews.md` § The File view's bars). Text opens
 * read-only in the code editor. An image, a video, an audio file or a PDF
 * shows in the web browser's own viewer, Markdown renders as a document, and
 * a file the page cannot show is a card (`file-previews.md`). The file's bar
 * offers Preview and Source for Markdown and SVG, Copy of a text file's
 * whole text, and Download.
 *
 * The tree docks, hides and shows over the file as its frame (`TreeFrame`)
 * decides; the host keeps whether it is open, its width, and the Preview or
 * Source choice (v-model), so they hold across files. A text file too large
 * to read whole shows its start, with a line saying how much of it shows and
 * Download (`file-previews.md` § Getting the bytes). A click on another file
 * in the tree, a pick from a crumb's menu, or a link in a Markdown document
 * asks the host to show it here; Back and Forward before the crumbs ask for
 * the files shown before and after. A path typed into the crumb row opens
 * that file, or finds that folder in the tree and selects it there; a
 * relative one starts from the workspace. Files
 * dropped on the tree, or uploaded from its menu, list under it until
 * cleared (`FileTree`).
 *
 * The file shows as its source keeps it (`plugin-pages.md` § What the
 * service keeps): a file seen a moment ago shows at once, scrolled where it
 * was, and a new text replaces the old in place; a file that is not text
 * keeps its card while it is checked. A read that fails keeps what shows and
 * says so quietly, with Retry; a file the Host cannot watch has the view say
 * that it shows it as last read, with Refresh.
 */
const props = defineProps<{
  source: FileBrowserSource
  /** The workspace: the tree's root, and where a relative path typed into the crumb row starts. */
  root: string
  /** What the tree's caption says in place of the root directory's name. */
  rootName?: string
  /** The file, by absolute path. */
  path: string | null
  /**
   * The line the file was opened at, 1-based, which the view scrolls to and
   * marks for a moment as a search result is, in the file's text: Markdown
   * and SVG show their source for it, as GitHub shows a file's plain text
   * for a link to a line of it, until Preview is chosen.
   */
  line?: number | null
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

const kind = computed(() => props.path === null ? 'text' : previewKind(props.path))
const sourceView = computed(() => props.path !== null && hasSourceView(props.path))
/** The source shows for the line the file was opened at, over the Preview chosen, until another choice. */
const sourceForLine = ref(false)
watch(() => [props.path, props.line], () => {
  sourceForLine.value = props.line != null
}, { immediate: true })
/** What Markdown and SVG show: the choice, or their source for a line. */
const shownMode = computed<'preview' | 'source'>({
  get: () => sourceForLine.value ? 'source' : mode.value,
  set: (chosen) => {
    sourceForLine.value = false
    mode.value = chosen
  },
})
/** An image, video, audio file or PDF shown from its bytes, SVG only while Preview shows. */
const media = computed(() => {
  const shown = kind.value
  if (shown === 'text' || shown === 'markdown' || !props.source.contents)
    return null
  return sourceView.value && shownMode.value === 'source' ? null : shown
})

type TextState =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'ready'; text: string }
  /** A file the text read cannot show; a text file too large for it shows its start instead. */
  | { phase: 'card'; tooLarge: boolean }
  /** Nothing is at the path. */
  | { phase: 'missing' }
  | { phase: 'failed'; message: string; requestId: string | null }

/**
 * The folder of the file shown, whose listing says whether the file is a
 * symbolic link whose target is gone or cannot be read: such a link says so
 * in the file's place from the listing, whatever the read of the file
 * answers (`file-previews.md` § Symbolic links). The listing and the file
 * are asked for at once, so a file whose folder is not kept yet costs no
 * extra round trip; the tree's own listing, kept, when the file was opened
 * there.
 */
const folder = useShowing(
  () => props.source,
  () => props.path === null ? null : parentPath(props.path),
  (source, path) => source.showListing(path),
)
/** The file as its folder's listing names it; undefined before the listing, or when it does not name it. */
const listed = computed(() => {
  const path = props.path
  return path === null ? undefined : folder.entry.value?.value?.find((entry) => entry.name === baseName(path))
})
/** The file, when it is a link whose target cannot be reached, as its folder's listing says. */
const brokenLink = computed(() => listed.value?.link?.broken ? listed.value.link : null)
/** Whether the listing names the path a folder now, as a link opened while its target was gone may come to: nothing to read as a file. */
const leadsToFolder = computed(() => listed.value?.isDirectory === true)

const shown = useShowing(
  () => props.source,
  () => props.path !== null && media.value === null && !leadsToFolder.value ? props.path : null,
  (source, path) => source.showText?.(path),
)

const state = computed<TextState>(() => {
  // A broken link's read goes on, for Retry, but what it answered does not show.
  if (props.path === null || media.value !== null || leadsToFolder.value || brokenLink.value !== null)
    return { phase: 'idle' }
  if (!props.source.showText)
    return { phase: 'failed', message: 'This source cannot read files.', requestId: null }
  const entry = shown.entry.value
  if (!entry)
    return { phase: 'loading' }
  // Each answer is drawn as itself, and stays while it is checked.
  switch (entry.answer) {
    case 'value':
      return entry.value === undefined ? { phase: 'loading' } : { phase: 'ready', text: entry.value.text }
    case 'binary':
    case 'too-large':
      return { phase: 'card', tooLarge: entry.answer === 'too-large' }
    case 'gone':
      return { phase: 'missing' }
    case null:
      return entry.failure
        ? { phase: 'failed', message: entry.failure.message ?? 'The file could not be read.', requestId: entry.failure.requestId ?? null }
        : { phase: 'loading' }
  }
})

/** How many times the text shown was read again successfully: a document's images that did not load try again then. */
const reloads = ref(0)
watch(() => shown.entry.value?.reading, (reading, wasReading) => {
  if (wasReading && !reading && !shown.entry.value?.failure)
    reloads.value += 1
})

/** Why the last read of the file shown could not be made, while what it answered before stays. */
const staleBecause = computed(() => {
  const entry = shown.entry.value
  return entry?.answer && entry.failure && brokenLink.value === null ? entry.failure.message ?? 'The read failed.' : null
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

/** The image or video shown, whose read Retry repeats. */
const preview = ref<InstanceType<typeof FilePreview> | null>(null)

/** Retry under a broken link's sentence: its folder, which said so, and the file, as the view reads it. */
async function retryLink(): Promise<void> {
  await Promise.all([folder.retry(), media.value === null ? shown.retry() : preview.value?.retry()])
}

/**
 * What the region says while it shows no file: none chosen, a read on its
 * way, a link whose target is gone or cannot be read, that nothing is at
 * the path, or why the read failed. Retry stands under the failures and
 * reads again what said so, the link's folder for a link, and the region
 * loads until its read ends.
 */
const placeholder = computed(() => {
  const current = state.value
  if (brokenLink.value)
    return { status: 'failed' as const, icon: MISSING_FILE.icon, label: `${linkDescription(brokenLink.value)}.`, detail: null, requestId: null, retry: retryLink }
  if (current.phase === 'missing')
    return { status: 'failed' as const, icon: MISSING_FILE.icon, label: MISSING_FILE.label, detail: null, requestId: null }
  if (current.phase === 'failed')
    return { status: 'failed' as const, icon: undefined, label: 'Could not read this file.', detail: current.message, requestId: current.requestId }
  if (current.phase === 'idle')
    return { status: 'note' as const, icon: undefined, label: 'Select a file.', detail: null, requestId: null }
  return { status: 'loading' as const, icon: undefined, label: 'Reading…', detail: null, requestId: null }
})

/** What the media preview of a file reports it shows, the image's zoom included. */
const mediaShown = ref<{ path: string; shown: ShownFile } | null>(null)
/** A card's file, described for the size its bar says. */
const card = useShowing(
  () => props.source.contents,
  () => state.value.phase === 'card' ? props.path : null,
  (contents, path) => contents.showDescription(path),
)

/** What the view shows of its file, which the file's bar names and acts on. */
const shownFile = computed<ShownFile>(() => {
  if (brokenLink.value !== null)
    return { as: 'missing' }
  if (media.value !== null)
    return mediaShown.value?.path === props.path ? mediaShown.value.shown : { as: 'reading' }
  const current = state.value
  switch (current.phase) {
    case 'ready':
      return { as: 'text', text: current.text }
    case 'missing':
      return { as: 'missing' }
    case 'card':
      if (start.value?.phase === 'ready')
        return { as: 'start', size: start.value.size }
      return start.value?.phase === 'loading'
        ? { as: 'reading' }
        : { as: 'card', size: card.entry.value?.value?.size ?? null }
    default:
      return { as: 'reading' }
  }
})
/** The file's bar; Download only where the source serves files' bytes. */
const bar = computed(() => {
  if (props.path === null)
    return null
  const shown = fileBar(props.path, shownFile.value)
  return { ...shown, download: shown.download && props.source.contents !== undefined }
})

/** Where each file this view showed was scrolled to, to open there again. */
const scrolls = new Map<string, StateEffect<unknown>>()

/** Rendered Markdown, unless Source is chosen or the document is too large to render. */
const markdown = computed(() => kind.value === 'markdown' &&
  shownMode.value === 'preview' &&
  state.value.phase === 'ready' &&
  renderable(state.value.text))
const tooLargeToRender = computed(() => kind.value === 'markdown' &&
  (state.value.phase === 'ready' ? !renderable(state.value.text) : start.value?.phase === 'ready'))
const place = computed<DocumentPlace | null>(() => props.path === null ? null : { path: props.path, root: props.root })

// A directory typed into the crumb row: selected in the tree until another file opens.
const located = ref<string | null>(null)
const frame = ref<InstanceType<typeof TreeFrame> | null>(null)
const treeView = ref<InstanceType<typeof FileTree> | null>(null)
let going: AbortController | null = null

watch(() => props.path, () => {
  located.value = null
})

// A file the tree opened, whose link the tree has just read again.
let openedFromTree: string | null = null

/**
 * A link opened, from a message, Back, the crumbs or the tree, is a link
 * used (`file-previews.md` § Symbolic links): its folder, when kept, is
 * listed again, unless the tree has just done so to open it, and the file
 * is read again, at once, since neither the folder's watch nor the file's
 * sees its target come or go; so a link its listing called missing whose
 * target is back shows the file. A read on its way is joined, not repeated.
 */
function readOpenedLink(listFolder: boolean): void {
  if (props.path === null || listed.value?.link === undefined)
    return
  if (listFolder)
    void folder.retry()
  void (media.value === null ? shown.retry() : preview.value?.retry())
}

// Another file shown is one opened.
watch(() => props.path, (path) => {
  const fromTree = path !== null && path === openedFromTree
  openedFromTree = null
  readOpenedLink(!fromTree)
}, { immediate: true, flush: 'post' })

/**
 * A file opened from the view itself, its tree, crumbs or a document's
 * link: the file shown now, opened again, changes no path, so its link is
 * read again here; `folderListed`, when the tree listed its folder again.
 */
function openHere(path: string, folderListed = false): void {
  if (path === props.path) {
    readOpenedLink(!folderListed)
    return
  }
  if (folderListed)
    openedFromTree = path
  emit('open', path)
}

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
 * is not there says that it does not exist.
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
    openHere(target)
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
  // The tree lists a link's folder again itself before it opens the link.
  openHere(path, true)
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
        :base="root"
        :source="source"
        :leaf="path ? 'file' : 'directory'"
        @navigate="go"
        @open="openHere($event)"
      />
    </template>
    <template #default="{ overlap }">
    <div class="flex h-full min-h-0 flex-col">
      <FileBar
        v-if="bar"
        v-model:mode="shownMode"
        :bar="bar"
        :too-large-to-render="tooLargeToRender"
        :overlap="overlap"
        @download="download"
      />
      <RegionNote
        v-if="shown.entry.value?.answer && shown.entry.value.unwatched && !brokenLink"
        :icon="History"
        label="Showing this file as it was last read."
        :detail="shown.entry.value.unwatched"
        action="Refresh"
        @action="shown.retry"
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
        <!-- Kept while a broken link's sentence shows, its read Retry's to repeat. -->
        <FilePreview
          v-if="media && path && source.contents && !leadsToFolder"
          v-show="!brokenLink"
          ref="preview"
          :key="path"
          :path="path"
          :kind="media"
          :contents="source.contents"
          :caption="false"
          @shown="mediaShown = { path, shown: $event }"
        />
        <template v-if="!(media && path && source.contents && !leadsToFolder) || brokenLink">
        <MarkdownDocument
          v-if="markdown && state.phase === 'ready' && place"
          :text="state.text"
          :place="place"
          :contents="source.contents"
          :reloads="reloads"
          @open="openHere($event)"
        />
        <!-- One editor per file: a new text of it replaces the old in place. -->
        <CodeEditor
          v-else-if="(state.phase === 'ready' || start?.phase === 'ready') && path"
          :key="path"
          class="h-full"
          :path="path"
          :text="state.phase === 'ready' ? state.text : start?.phase === 'ready' ? start.text : ''"
          :scroll-to="scrolls.get(path)"
          :line="line"
          @left="(left, snapshot) => scrolls.set(left, snapshot)"
        />
        <FileSummary
          v-else-if="state.phase === 'card' && start?.phase !== 'loading' && path"
          :path="path"
          :contents="source.contents"
          :note="state.tooLarge ? TOO_LARGE_NOTE : null"
        />
        <!-- It centres in the space beside a tree shown over the view. -->
        <div v-else class="h-full" :style="{ paddingInlineEnd: `${overlap}px` }">
          <RegionStatus
            class="h-full"
            :status="placeholder.status"
            :icon="placeholder.icon"
            :label="placeholder.label"
            loading-label="Reading…"
            :detail="placeholder.detail"
            :request-id="placeholder.requestId"
            :on-retry="placeholder.retry ?? shown.retry"
          />
        </div>
        </template>
      </div>
    </div>
    </template>
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

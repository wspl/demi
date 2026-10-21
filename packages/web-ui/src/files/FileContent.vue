<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { FileText, History, RefreshCw } from '@lucide/vue'
import type { StateEffect } from '@codemirror/state'
import { CodeEditor, useEditor } from '../editor/load'
import { regionForm, regionsChanged } from '../infra/region-wait'
import { t as editorText } from '../editor/locales/text'
import { renderable, type DocumentPlace } from '../markdown/document'
import RegionNote from '../ui/RegionNote.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import FileBar from './FileBar.vue'
import FilePreview from './FilePreview.vue'
import FileSummary from './FileSummary.vue'
import MarkdownDocument from './MarkdownDocument.vue'
import { downloadUrl } from './download'
import { fileBar, type ShownFile } from './file-bar'
import { formatByteRatio, linkSentence } from './format'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import { baseName } from '@demicodes/utils'
import { parentPath } from './paths'
import { hasSourceView, missingFile, previewKind, tooLargeNote } from './preview'
import { useShowing } from './showing'
import { useTextStart } from './text-start'
import type { FileBrowserSource } from './types'

/**
 * One file of a source as the File view shows it, under the bar that says what
 * it is and holds what can be done with it.
 *
 * What it shows:
 *
 * - Text, read-only in the code editor.
 * - An image, a video, an audio file or a PDF in the web browser's own viewer.
 * - Markdown rendered as a document.
 * - A card for a file the page cannot show.
 *
 * The bar offers Preview and Source for Markdown and SVG, Copy of a text file's
 * whole text, and Download. `label`, when given, says what the file is in its
 * place, as an attachment's preview names its file and size.
 *
 * The File view puts it beside its tree, and an attachment's preview shows it
 * alone over the attachment's copy (`AttachmentView`). A link in a Markdown
 * document asks the host to open its file (`open`).
 *
 * The file shows as its source keeps it:
 *
 * - A file seen a moment ago shows at once, scrolled where it was, and a new
 *   text replaces the old in place.
 * - A file that is not text keeps its card while it is checked.
 * - A read that fails keeps what shows and says so quietly, with Retry.
 * - A file the Host cannot watch has the view say that it shows it as last read,
 *   with Refresh.
 * - A text file too large to read whole shows its start, with a line saying how
 *   much of it shows and Download.
 */
const props = withDefaults(defineProps<{
  source: FileBrowserSource
  /** The directory a Markdown document's relative links and images start from. */
  root: string
  /** The file, by absolute path. */
  path: string | null
  /**
   * The line the file was opened at, 1-based, which the view scrolls to and marks
   * for a moment as a search result is, in the file's text. Markdown and SVG show
   * their source for it, as GitHub shows a file's plain text for a link to a line
   * of it, until Preview is chosen.
   */
  line?: number | null
  /** The width at the end that a tree shown over the file covers: the bar's controls and the centred states stay beside it. */
  overlap?: number
  /** What the bar says on its left in place of the file's kind and facts. */
  label?: string
}>(), { line: null, overlap: 0, label: undefined })

const emit = defineEmits<{ open: [path: string] }>()

// The view shows its loading state until the editor is there, whose load starts now.
const { load: editorLoad, retry: retryEditor } = useEditor()
const editorReady = computed(() => editorLoad.value.phase === 'ready')
/** The path of the file whose editor is built, which draws its text with its colors. */
const builtPath = ref<string | null>(null)

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
 * symbolic link whose target is gone or cannot be read. Such a link says so in
 * the file's place from the listing, whatever the read of the file answers.
 *
 * The listing and the file are asked for at once, so a file whose folder is not
 * kept yet costs no extra round trip. When the file was opened from the tree,
 * the tree's own listing, kept, answers.
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
    return { phase: 'failed', message: t('view.noSource'), requestId: null }
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
        ? { phase: 'failed', message: entry.failure.message ?? t('view.readFailedDetail'), requestId: entry.failure.requestId ?? null }
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
  return entry?.answer && entry.failure && brokenLink.value === null ? entry.failure.message ?? t('view.readAgainFailed') : null
})

/** The start of a text file too large for the text read, while one is shown. */
const start = useTextStart(
  () => props.source.contents,
  () => state.value.phase === 'card' && state.value.tooLarge ? props.path : null,
)
/** The line over a large file's start: how much of how much shows. */
const startNote = computed(() => start.value?.phase === 'ready'
  ? t('view.startOf', { ratio: formatByteRatio(start.value.shown, start.value.size) })
  : null)

/** The image or video shown, whose read Retry repeats. */
const preview = ref<InstanceType<typeof FilePreview> | null>(null)

/** Retry under a broken link's sentence: its folder, which said so, and the file, as the view reads it. */
async function retryLink(): Promise<void> {
  await Promise.all([folder.retry(), media.value === null ? shown.retry() : preview.value?.retry()])
}

/**
 * What the region says while it shows no file:
 *
 * - The editor is on its way or failed.
 * - None is chosen.
 * - A read is on its way.
 * - A link's target is gone or cannot be read.
 * - Nothing is at the path.
 * - The read failed, and why.
 *
 * Retry stands under the failures and loads again what said so: the editor, or
 * the link's folder for a link. The region loads until that ends.
 */
const placeholder = computed(() => {
  const current = state.value
  const editor = editorLoad.value
  if (editor.phase === 'failed')
    return { status: 'failed' as const, icon: undefined, label: editorText('unavailable'), detail: editor.message, requestId: null, retry: retryEditor }
  if (editor.phase === 'loading')
    return { status: 'loading' as const, icon: undefined, label: t('reading'), detail: null, requestId: null }
  if (brokenLink.value)
    return { status: 'failed' as const, icon: missingFile().icon, label: linkSentence(brokenLink.value), detail: null, requestId: null, retry: retryLink }
  if (current.phase === 'missing') {
    const missing = missingFile()
    return { status: 'failed' as const, icon: missing.icon, label: missing.label, detail: null, requestId: null }
  }
  if (current.phase === 'failed')
    return { status: 'failed' as const, icon: undefined, label: t('view.readFailed'), detail: current.message, requestId: current.requestId }
  if (current.phase === 'idle')
    return { status: 'note' as const, icon: undefined, label: t('view.selectFile'), detail: null, requestId: null }
  return { status: 'loading' as const, icon: undefined, label: t('reading'), detail: null, requestId: null }
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
/**
 * The file's bar, its label in place of the kind and facts when given; Download only where the source serves files' bytes.
 * Where the file's text is known the bar stands before the editor is there, without the language's name, so the text does not move when the editor arrives.
 */
const bar = computed(() => {
  const editor = editorLoad.value
  const textKnown = shownFile.value.as === 'text' || shownFile.value.as === 'start'
  if (props.path === null || editor.phase === 'failed' || (editor.phase !== 'ready' && !textKnown))
    return null
  const shown = fileBar(props.path, shownFile.value, editor.phase === 'ready' ? editor.chunk.languageName : null)
  return {
    ...shown,
    ...(props.label === undefined || !shown.download ? {} : { label: props.label }),
    download: shown.download && props.source.contents !== undefined,
  }
})

/** Where each file this view showed was scrolled to, to open there again. */
const scrolls = new Map<string, StateEffect<unknown>>()

/** Rendered Markdown, unless Source is chosen or the document is too large to render. */
const markdown = computed(() => kind.value === 'markdown' &&
  shownMode.value === 'preview' &&
  state.value.phase === 'ready' &&
  renderable(state.value.text))
/** Whether the view shows the file's text in the editor, unless the editor failed to load, which the region's note says. */
const editorShown = computed(() => !markdown.value && props.path !== null && editorLoad.value.phase !== 'failed' && (state.value.phase === 'ready' || start.value?.phase === 'ready'))
/** The text the editor shows. */
const shownText = computed(() => state.value.phase === 'ready' ? state.value.text : start.value?.phase === 'ready' ? start.value.text : '')
/**
 * How the file's text draws: its region (`infra/region-wait.ts`) needs the
 * editor and the file's language, so for 300 ms at most, in all, it draws
 * nothing, then the text without colors, which the editor replaces once built.
 */
const editorForm = computed(() => {
  const path = props.path
  if (!editorShown.value || path === null)
    return 'final'
  return regionForm(`editor:${path}`, () => builtPath.value === path)
})
function editorBuilt(path: string): void {
  builtPath.value = path
  regionsChanged()
}
const tooLargeToRender = computed(() => kind.value === 'markdown' &&
  (state.value.phase === 'ready' ? !renderable(state.value.text) : start.value?.phase === 'ready'))
const place = computed<DocumentPlace | null>(() => props.path === null ? null : { path: props.path, root: props.root })

/**
 * A link opened, from a message, Back, the crumbs or the tree, is a link used.
 *
 * Its folder, when kept, is listed again, unless the tree has just done so to
 * open it. The file is read again at once, since neither the folder's watch nor
 * the file's sees its target come or go. So a link whose listing called it
 * missing, and whose target is back, shows the file. A read on its way is
 * joined, not repeated.
 */
function readOpenedLink(listFolder: boolean): void {
  if (props.path === null || listed.value?.link === undefined)
    return
  if (listFolder)
    void folder.retry()
  void (media.value === null ? shown.retry() : preview.value?.retry())
}

// The file shown first is one opened.
onMounted(() => readOpenedLink(true))

function download(): void {
  if (props.path !== null && props.source.contents)
    downloadUrl(props.source.contents.url(props.path, { download: true }), baseName(props.path))
}

defineExpose({ readOpenedLink })
</script>

<template>
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
    :label="t('view.notWatched')"
    :detail="shown.entry.value.unwatched"
    :action="t('actions.refresh')"
    @action="shown.retry"
  />
  <RegionNote
    v-if="staleBecause"
    :icon="RefreshCw"
    :label="t('view.refreshFailed')"
    :detail="staleBecause"
    :action="shared('common.retry')"
    @action="shown.retry"
  />
  <RegionNote v-if="startNote" :icon="FileText" :label="startNote" :action="t('actions.download')" @action="download" />
  <div class="min-h-0 flex-1">
    <!-- Kept while a broken link's sentence shows, its read Retry's to repeat. -->
    <FilePreview
      v-if="editorReady && media && path && source.contents && !leadsToFolder"
      v-show="!brokenLink"
      ref="preview"
      :key="path"
      :path="path"
      :kind="media"
      :contents="source.contents"
      :caption="false"
      @shown="mediaShown = { path, shown: $event }"
    />
    <template v-if="!(editorReady && media && path && source.contents && !leadsToFolder) || brokenLink">
    <MarkdownDocument
      v-if="editorReady && markdown && state.phase === 'ready' && place"
      :text="state.text"
      :place="place"
      :contents="source.contents"
      :reloads="reloads"
      @open="emit('open', $event)"
    />
    <!-- One editor per file: a new text of it replaces the old in place. -->
    <div v-else-if="editorShown && path" class="relative h-full">
      <CodeEditor
        v-if="editorReady"
        :key="path"
        class="h-full"
        :path="path"
        :text="shownText"
        :scroll-to="scrolls.get(path)"
        :line="line"
        @left="(left, snapshot) => scrolls.set(left, snapshot)"
        @built="editorBuilt"
      />
      <!-- The text without colors, where the editor will stand, until it is built. -->
      <pre
        v-if="editorForm === 'plain'"
        class="absolute inset-0 m-0 overflow-hidden whitespace-pre bg-surface px-3 pt-3 font-mono text-[13px] leading-[20px] text-fg-body"
        data-plain-text
      >{{ shownText }}</pre>
    </div>
    <FileSummary
      v-else-if="editorReady && state.phase === 'card' && start?.phase !== 'loading' && path"
      :path="path"
      :contents="source.contents"
      :note="state.tooLarge ? tooLargeNote() : null"
    />
    <!-- It centres in the space beside a tree shown over the view. -->
    <div v-else class="h-full" :style="{ paddingInlineEnd: `${overlap}px` }">
      <RegionStatus
        class="h-full"
        :status="placeholder.status"
        :icon="placeholder.icon"
        :label="placeholder.label"
        :loading-label="t('reading')"
        :detail="placeholder.detail"
        :request-id="placeholder.requestId"
        :on-retry="placeholder.retry ?? shown.retry"
      />
    </div>
    </template>
  </div>
</div>
</template>

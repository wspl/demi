<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, Code, Download, Eye, FolderTree } from '@lucide/vue'
import CodeEditor from '../editor/components/CodeEditor.vue'
import { showToast } from '../infra/toast'
import { renderable, type DocumentPlace } from '../markdown/document'
import IconButton from '../ui/IconButton.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tooltip from '../ui/Tooltip.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FilePreview from './FilePreview.vue'
import FileSummary from './FileSummary.vue'
import FileTree from './FileTree.vue'
import MarkdownDocument from './MarkdownDocument.vue'
import { downloadUrl } from './download'
import { TREE_WIDTH } from './file-view'
import { baseName, normalizePath, parentPath } from './paths'
import { hasSourceView, previewKind, TOO_LARGE_NOTE } from './preview'
import { FileBrowserError, type FileBrowserSource } from './types'

/**
 * One file of a workspace, read through its source: the path as crumbs from
 * the workspace root, the file itself, and beside it the workspace tree with
 * the file selected. Text opens read-only in the code editor. An image, a
 * video, an audio file or a PDF shows in the browser's own viewer, Markdown
 * renders as a document, and a file the page cannot show is a card
 * (`file-previews.md`). Markdown and SVG offer Preview and Source, and every
 * file can be downloaded.
 *
 * The control at the end of the crumb row shows and hides the tree, and the
 * divider before the tree sizes it; the host keeps both, and the Preview or
 * Source choice (v-model), so they hold across files. A click on another file
 * in the tree, a pick from a crumb's menu, or a link in a Markdown document
 * asks the host to show it here; Back and Forward before the crumbs ask for
 * the files shown before and after. A path typed into the crumb row opens
 * that file, or finds that folder in the tree and selects it there.
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
  | { phase: 'card'; note: string | null }
  | { phase: 'failed'; message: string }

const state = ref<TextState>({ phase: 'loading' })
let controller: AbortController | null = null

/** Rendered Markdown, unless Source is chosen or the document is too large to render. */
const markdown = computed(() => kind.value === 'markdown' &&
  mode.value === 'preview' &&
  state.value.phase === 'ready' &&
  renderable(state.value.text))
const tooLargeToRender = computed(() => kind.value === 'markdown' &&
  state.value.phase === 'ready' &&
  !renderable(state.value.text))
const place = computed<DocumentPlace | null>(() => props.path === null
  ? null
  : {
      path: props.path,
      root: props.root,
      imageUrl: (path) => props.source.contents?.url(path) ?? '',
    })

async function read(): Promise<void> {
  controller?.abort()
  controller = null
  if (props.path === null || media.value !== null) {
    state.value = { phase: 'idle' }
    return
  }
  const current = new AbortController()
  controller = current
  state.value = { phase: 'loading' }
  if (!props.source.read) {
    state.value = { phase: 'failed', message: 'This source cannot read files.' }
    return
  }
  try {
    const text = await props.source.read(props.path, current.signal)
    if (current.signal.aborted)
      return
    state.value = { phase: 'ready', text }
  } catch (error) {
    if (current.signal.aborted)
      return
    // A file the text read cannot show is a card.
    if (error instanceof FileBrowserError && (error.kind === 'binary' || error.kind === 'too-large')) {
      state.value = { phase: 'card', note: error.kind === 'too-large' ? TOO_LARGE_NOTE : null }
      return
    }
    state.value = { phase: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}

// A directory typed into the crumb row: selected in the tree until another file opens.
const located = ref<string | null>(null)
const treeView = ref<InstanceType<typeof FileTree> | null>(null)
let going: AbortController | null = null

watch(() => props.path, () => {
  located.value = null
})

/** Whether `path` names a directory, by its parent's listing. */
async function isDirectory(path: string, signal: AbortSignal): Promise<boolean> {
  // The filesystem root has no parent to list.
  if (path === '/')
    return true
  if (!props.source.list)
    return false
  try {
    const entries = await props.source.list(parentPath(path), signal)
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
  const directory = await isDirectory(target, current.signal)
  // Another path was typed meanwhile, or the view went away.
  if (current.signal.aborted)
    return
  if (!directory) {
    emit('open', target)
    return
  }
  const root = normalizePath(props.root)
  if (target !== root && !target.startsWith(`${root}/`)) {
    showToast({ title: 'Not in this workspace', message: `The tree shows ${props.rootName ?? root} only.` })
    return
  }
  located.value = target
  tree.value = true
  await nextTick()
  treeView.value?.reveal(target)
}

function download(): void {
  if (props.path !== null && props.source.contents)
    downloadUrl(props.source.contents.url(props.path, { download: true }))
}

watch(() => [props.source, props.path, media.value], read, { immediate: true })

onBeforeUnmount(() => {
  controller?.abort()
  going?.abort()
})
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
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
      <Tooltip :content="tree ? 'Hide file tree' : 'Show file tree'" class="shrink-0">
        <IconButton
          :icon="FolderTree"
          variant="ghost"
          :pressed="tree"
          :aria-label="tree ? 'Hide file tree' : 'Show file tree'"
          @click="tree = !tree"
        />
      </Tooltip>
    </div>
    <div class="flex min-h-0 flex-1 border-t border-line">
      <div class="relative min-w-0 flex-1">
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
          @open="emit('open', $event)"
        />
        <!-- Every read passes through loading, so each text gets an editor of its own. -->
        <CodeEditor
          v-else-if="state.phase === 'ready' && path"
          class="h-full"
          :path="path"
          :text="state.text"
        />
        <FileSummary
          v-else-if="state.phase === 'card' && path"
          :path="path"
          :contents="source.contents"
          :note="state.note"
        />
        <RegionStatus
          v-else
          class="h-full"
          :busy="state.phase === 'loading'"
          :failed="state.phase === 'failed'"
          :label="state.phase === 'idle' ? 'Select a file.' : state.phase === 'loading' ? 'Reading…' : 'Could not read this file.'"
          :detail="state.phase === 'failed' ? state.message : null"
          :action="state.phase === 'failed' ? 'Retry' : undefined"
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
          label="File tree width"
        />
        <!-- The tree's width is the divider's; flex must not grow or shrink it. -->
        <FileTree
          ref="treeView"
          class="border-l border-line"
          :style="{ flex: `0 0 ${treeWidth}px`, width: `${treeWidth}px` }"
          :source="source"
          :root="root"
          :root-name="rootName"
          :selected="located ?? path"
          @open="emit('open', $event)"
        />
      </template>
    </div>
  </div>
</template>

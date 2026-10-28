<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight } from '@lucide/vue'
import { showToast } from '../infra/toast'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FileContent from './FileContent.vue'
import FileSearch from './FileSearch.vue'
import FileTree from './FileTree.vue'
import TreeFrame from './TreeFrame.vue'
import { TREE_WIDTH } from './file-view'
import { t } from './locales/text'
import { baseName } from '@demicodes/utils'
import { normalizePath, parentPath } from './paths'
import type { FileBrowserSource, FileDirectory } from './types'

/**
 * One file of a Host, read through its source: the whole path as crumbs from the
 * Host's root, the file itself under its bar (`FileContent`), and beside it the
 * file tree with the file selected.
 *
 * The tree docks, hides and shows over the file as its frame (`TreeFrame`)
 * decides. The host keeps whether it is open, its width, and the Preview or
 * Source choice (v-model), so they hold across the files it shows.
 *
 * What asks the host to show another file here:
 *
 * - A click on another file in the tree.
 * - A pick from a crumb's menu.
 * - A link in a Markdown document.
 *
 * Back and Forward before the crumbs ask the host to go back or forward, as a
 * web browser's do; the host says when it can.
 * A path typed into the crumb row opens that file, or finds that folder in the
 * tree and selects it there; a relative one starts from the root.
 *
 * Files dropped on the tree, or uploaded from its menu, list under it until
 * cleared (`FileTree`).
 *
 * A source that searches offers Search in Files in the tree's caption, which
 * floats a field over the view (`FileSearch`); a file it opens shows at the
 * line it was found at.
 */

const props = defineProps<{
  source: FileBrowserSource
  /** The tree's root, and where a relative path typed into the crumb row starts. */
  root: string
  /** What the tree's caption says in place of the root directory's name. */
  rootName?: string
  /** The directories the host can show, with the one shown: the tree's caption is a menu of them when that makes several. */
  directories?: readonly FileDirectory[]
  /** The file, by absolute path. */
  path: string | null
  /**
   * The line the file was opened at, 1-based, which the view scrolls to and marks
   * for a moment as a search result is, in the file's text. Markdown and SVG show
   * their source for it, as GitHub shows a file's plain text for a link to a line
   * of it, until Preview is chosen.
   */
  line?: number | null
  /** Whether the host can go back or forward from what this view shows. */
  canBack?: boolean
  canForward?: boolean
}>()

const emit = defineEmits<{
  /** A file to show, and the line to open it at, from Search in Files. */
  open: [path: string, line?: number]
  back: []
  forward: []
  /** The tree's caption chose another of `directories`. */
  switchDirectory: [directory: string]
}>()

const tree = defineModel<boolean>('tree', { default: true })

const treeWidth = defineModel<number>('treeWidth', { default: TREE_WIDTH.default })

/** Markdown and SVG only: their rendered view, or their text. */
const mode = defineModel<'preview' | 'source'>('mode', { default: 'preview' })

const content = ref<InstanceType<typeof FileContent> | null>(null)

/**
 * A directory typed into the crumb row, selected in the tree until another
 * file opens: the host's, as a place it shows, or the view's own without one.
 */
const located = defineModel<string | null>('folder', { default: null })
const frame = ref<InstanceType<typeof TreeFrame> | null>(null)
const treeView = ref<InstanceType<typeof FileTree> | null>(null)
let going: AbortController | null = null

watch(() => props.path, () => {
  located.value = null
})

/**
 * Show in Files: the tree, docked where it fits and over the file
 * otherwise, with the file's row selected, unfolded to and scrolled into
 * view.
 */
async function showInTree(): Promise<void> {
  const path = props.path
  if (path === null)
    return
  frame.value?.show()
  await nextTick()
  treeView.value?.reveal(path, false)
}

/** Whether Search in Files floats over the view. */
const searching = ref(false)

const searchField = ref<InstanceType<typeof FileSearch> | null>(null)

/** Search in Files: its field, focused, or focused again while it shows. */
function showSearch(): void {
  if (!props.source.search)
    return
  if (searching.value)
    searchField.value?.focus()
  else
    searching.value = true
}

/** A file Search in Files found: shown, at its line, the field and a tree over the file put away. */
function openFound(path: string, line: number | null): void {
  searching.value = false
  frame.value?.dismiss()
  // The file shown at that line already is scrolled to it again: nothing the owner holds changes.
  if (line !== null && path === props.path && line === props.line)
    content.value?.revealLine(line)
  emit('open', path, line ?? undefined)
}

defineExpose({ showInTree, showSearch })

// A file the tree opened, whose link the tree has just read again.
let openedFromTree: string | null = null

// Another file shown is one opened, as a link is used (`FileContent`'s
// `readOpenedLink`); the first, when the file's view mounts.
watch(() => props.path, (path) => {
  const fromTree = path !== null && path === openedFromTree
  openedFromTree = null
  content.value?.readOpenedLink(!fromTree)
}, { flush: 'post' })

/**
 * A file opened from the view itself, its tree, crumbs or a document's
 * link: the file shown now, opened again, changes no path, so its link is
 * read again here; `folderListed`, when the tree listed its folder again.
 */
function openHere(path: string, folderListed = false): void {
  if (path === props.path) {
    content.value?.readOpenedLink(!folderListed)
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
    showToast({ title: t('view.notInFolderTitle'), message: t('view.notInFolder', { name: props.rootName ?? root }), tone: 'neutral' })
    return
  }
  located.value = target
}

// The folder located, by the crumb row or by the host showing it again, is found in the tree.
watch(located, async (folder) => {
  if (folder === null)
    return
  frame.value?.show()
  await nextTick()
  treeView.value?.reveal(folder, true)
}, { immediate: true, flush: 'post' })

function openFromTree(path: string): void {
  frame.value?.dismiss()
  // The tree lists a link's folder again itself before it opens the link.
  openHere(path, true)
}

onBeforeUnmount(() => {
  going?.abort()
})
</script>

<template>
  <!-- Search in Files floats over the whole view. -->
  <div data-file-view class="relative flex h-full min-h-0 min-w-0 flex-col">
    <TreeFrame ref="frame" class="min-h-0 flex-1" v-model:open="tree" v-model:width="treeWidth" tree="files">
      <template #header>
        <!-- Back and Forward move through the files this view has shown. -->
        <div class="flex shrink-0 items-center">
          <Tooltip :content="t('actions.back')">
            <IconButton :icon="ArrowLeft" variant="ghost" :aria-label="t('actions.back')" :disabled="!canBack" @click="emit('back')" />
          </Tooltip>
          <Tooltip :content="t('actions.forward')">
            <IconButton :icon="ArrowRight" variant="ghost" :aria-label="t('actions.forward')" :disabled="!canForward" @click="emit('forward')" />
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
        <FileContent
          ref="content"
          v-model:mode="mode"
          :source="source"
          :root="root"
          :path="path"
          :line="line"
          :overlap="overlap"
          @open="openHere($event)"
        />
      </template>
      <template #tree>
        <FileTree
          ref="treeView"
          :source="source"
          :root="root"
          :root-name="rootName"
          :directories="directories"
          :home="source.home"
          :selected="located ?? path"
          :searchable="source.search !== undefined"
          @open="openFromTree"
          @switch="emit('switchDirectory', $event)"
          @search="showSearch"
        />
      </template>
    </TreeFrame>
    <FileSearch v-if="searching" ref="searchField" :source="source" :root="root" @close="searching = false" @open="openFound" />
  </div>
</template>

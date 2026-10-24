<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { FileView, PaneReading, treeLayout, usePage, usePaneReady, type TabUpdate } from '@demicodes/plugin-sdk'
import { fileView, locateFolder, searchShown, shownPath, showDirectory, showFile, showMode, treeShown, type FileData } from './data'

/**
 * The File view of one conversation: one file of its Host beside the Host's
 * tree, over the conversation's files service. A file picked in the tree or a
 * link in a Markdown preview replaces the file, and a folder the crumb row
 * goes to is found in the tree, each a step of the web browser's history,
 * which the view's Back and Forward take while they stay in the
 * conversation's panel. Until the conversation's working directory is known,
 * which its opening brings, the pane says it reads.
 */
const props = defineProps<{
  conversation: string
  tabId: string
  data: FileData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: FileData, how?: TabUpdate] }>()

const page = usePage()
const files = page.files(props.conversation)
const workspace = computed(() => files.workspace)
usePaneReady(() => workspace.value !== null)

/** The directory the view was opened for, whose tree it shows: the conversation's unless a child's link opened it. */
const root = computed(() => props.data.root ?? workspace.value?.root ?? '/')

/** A file picked in the tree, a link in a document or one Search in Files found, at its line, in the directory the view was opened for. */
function open(path: string, line?: number): void {
  emit('update', showFile(props.data, path, line, props.data.root))
}

/** A directory the tree's caption chose among the conversation's agents'. */
function switchDirectory(directory: string): void {
  emit('update', showDirectory(props.data, directory, workspace.value?.root ?? null))
}

/** A folder the crumb row went to; the view lets go of one as another file opens, which says so itself. */
function locate(folder: string | null): void {
  if (folder !== null) {
    emit('update', locateFolder(props.data, folder))
  }
}

/** Preview or Source, chosen here: the user's choice from now on, and this file's in the address. */
function choose(chosen: 'preview' | 'source'): void {
  fileView.value = chosen
  emit('update', showMode(props.data, chosen), { replace: true })
}

const view = ref<InstanceType<typeof FileView> | null>(null)
// Show in Files: once the view shows the file, its tree shows it too. After the render, so the view has the file.
watch([() => props.data.tree, view], ([tree, shown]) => {
  if (!tree || !shown) {
    return
  }
  emit('update', treeShown(props.data), { replace: true })
  void shown.showInTree()
}, { immediate: true, flush: 'post' })
// Search in Files: once the view is there, its field floats over it, focused.
watch([() => props.data.search, view], ([search, shown]) => {
  if (!search || !shown) {
    return
  }
  emit('update', searchShown(props.data), { replace: true })
  shown.showSearch()
}, { immediate: true, flush: 'post' })
</script>

<template>
  <FileView
    v-if="workspace"
    ref="view"
    v-model:tree="treeLayout.open.value"
    v-model:tree-width="treeLayout.width.value"
    :mode="data.view ?? fileView"
    :folder="data.folder ?? null"
    :source="workspace.source"
    :root="root"
    :root-name="data.root === undefined ? workspace.name : undefined"
    :directories="files.directories"
    :path="data.path ? shownPath(root, data.path) : null"
    :line="data.line ?? null"
    :can-back="page.panel.canGo(conversation, 'back')"
    :can-forward="page.panel.canGo(conversation, 'forward')"
    @update:mode="choose"
    @update:folder="locate"
    @open="open"
    @switch-directory="switchDirectory"
    @back="page.panel.go('back')"
    @forward="page.panel.go('forward')"
  />
  <div v-else class="relative flex-1">
    <PaneReading />
  </div>
</template>

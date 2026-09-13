<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, FolderTree } from '@lucide/vue'
import CodeEditor from '../editor/components/CodeEditor.vue'
import { appEditorHost } from '../editor/host/appHost'
import { toEditorUri } from '../editor/editorUri'
import IconButton from '../ui/IconButton.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import Tooltip from '../ui/Tooltip.vue'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FileTree from './FileTree.vue'
import { TREE_WIDTH } from './file-view'
import { FileBrowserError, type FileBrowserSource } from './types'

/**
 * One file of a workspace, read through its source: the path as crumbs from
 * the workspace root, the text in the code editor (read-only for now: the
 * editor can edit, the product does not yet save), and beside it the
 * workspace tree with the file selected. The control at the end of the crumb row shows and
 * hides the tree, and the divider before the tree sizes it; the host keeps
 * both (v-model) so they hold across files. A click on another file in the
 * tree, or a pick from a crumb's menu, asks the host to show it here; Back
 * and Forward before the crumbs ask for the files shown before and after.
 */
const props = defineProps<{
  source: FileBrowserSource
  /** The workspace: the tree's root and the first crumb. */
  root: string
  /** The file, by absolute path. */
  path: string
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

const editor = ref<InstanceType<typeof CodeEditor> | null>(null)
const state = ref<'loading' | 'ready' | 'failed'>('loading')
const failure = ref<string | null>(null)
let controller: AbortController | null = null

async function read(): Promise<void> {
  controller?.abort()
  const current = new AbortController()
  controller = current
  state.value = 'loading'
  failure.value = null
  if (!props.source.read) {
    state.value = 'failed'
    failure.value = 'This source cannot read files.'
    return
  }
  try {
    const content = await props.source.read(props.path, current.signal)
    if (current.signal.aborted) {
      return
    }
    state.value = 'ready'
    await nextTick()
    editor.value?.setFile({ resourceUri: toEditorUri('workspace', props.path), content })
  } catch (error) {
    if (current.signal.aborted) {
      return
    }
    state.value = 'failed'
    failure.value = error instanceof FileBrowserError || error instanceof Error
      ? error.message
      : String(error)
  }
}

watch(() => [props.source, props.path], read, { immediate: true })

onBeforeUnmount(() => {
  controller?.abort()
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
        class="min-w-0 flex-1"
        mode="browse"
        :path="path"
        :root="root"
        :source="source"
        leaf="file"
        :editable="false"
        @open="emit('open', $event)"
      />
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
        <CodeEditor
          v-if="state === 'ready'"
          ref="editor"
          class="h-full"
          :host="appEditorHost"
          read-only
        />
        <RegionStatus
          v-else
          class="h-full"
          :busy="state === 'loading'"
          :failed="state === 'failed'"
          :label="state === 'loading' ? 'Reading…' : 'Could not read this file.'"
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
          label="File tree width"
        />
        <!-- The tree's width is the divider's; flex must not grow or shrink it. -->
        <FileTree
          class="border-l border-line"
          :style="{ flex: `0 0 ${treeWidth}px`, width: `${treeWidth}px` }"
          :source="source"
          :root="root"
          :selected="path"
          @open="emit('open', $event)"
        />
      </template>
    </div>
  </div>
</template>

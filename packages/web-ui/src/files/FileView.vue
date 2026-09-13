<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { FolderTree } from '@lucide/vue'
import CodePreview from '../ui/CodePreview.vue'
import IconButton from '../ui/IconButton.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Tooltip from '../ui/Tooltip.vue'
import { getLanguageFromPath } from '../markdown/highlight'
import FileBrowserAddressBar from './FileBrowserAddressBar.vue'
import FileTree from './FileTree.vue'
import { FileBrowserError, type FileBrowserSource } from './types'

/**
 * One file of a workspace, read through its source: the path as crumbs from
 * the workspace root, the highlighted text, and beside it the workspace tree
 * with the file selected. The control at the end of the crumb row shows and
 * hides the tree; the host keeps that choice (v-model) so it holds across
 * files. A click on another file in the tree asks the host to open it.
 */
const props = defineProps<{
  source: FileBrowserSource
  /** The workspace: the tree's root and the first crumb. */
  root: string
  /** The file, by absolute path. */
  path: string
}>()

const emit = defineEmits<{
  open: [path: string]
}>()

const tree = defineModel<boolean>('tree', { default: true })

const text = ref('')
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
    text.value = content
    state.value = 'ready'
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

const lang = computed(() => getLanguageFromPath(props.path))
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <FileBrowserAddressBar
        class="min-w-0 flex-1"
        :path="path"
        :root="root"
        :source="source"
        leaf="file"
        :editable="false"
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
        <CodePreview v-if="state === 'ready'" :code="text" :lang="lang" />
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
      <FileTree
        v-if="tree"
        class="w-48 shrink-0 border-l border-line"
        :source="source"
        :root="root"
        :selected="path"
        @open="emit('open', $event)"
      />
    </div>
  </div>
</template>

<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: whether the view shows them rendered or as source holds
// across files and conversations for the page's lifetime.
const mode = ref<'preview' | 'source'>('preview')
</script>

<script setup lang="ts">
import { computed } from 'vue'
import { FileView, joinPath, treeLayout, usePage } from '@demicodes/plugin-sdk'
import { goBack, goForward, showFile, type FileData } from './data'

/**
 * The File view of one conversation (`file-previews.md`): one file of its
 * Host beside the Host's tree, over the conversation's files service. A file
 * picked in the tree or a link in a Markdown preview replaces the file, and
 * Back returns to it.
 */
const props = defineProps<{
  conversation: string
  tabId: string
  data: FileData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: FileData] }>()

const files = usePage().files(props.conversation)
const workspace = computed(() => files.workspace)

function absolute(root: string, path: string): string {
  return path.startsWith('/') ? path : joinPath(root, path)
}

/** The directory the view was opened for, whose tree it shows: the conversation's unless a child's link opened it. */
const root = computed(() => props.data.root ?? workspace.value?.root ?? '/')

/** A file picked in the tree or a link in a document, in the directory the view was opened for. */
function open(path: string): void {
  emit('update', showFile(props.data, path, undefined, props.data.root))
}
</script>

<template>
  <FileView
    v-if="workspace"
    v-model:tree="treeLayout.open.value"
    v-model:tree-width="treeLayout.width.value"
    v-model:mode="mode"
    :source="workspace.source"
    :root="root"
    :root-name="data.root === undefined ? workspace.name : undefined"
    :path="data.path ? absolute(root, data.path) : null"
    :line="data.line ?? null"
    :can-back="data.back.length > 0"
    :can-forward="data.forward.length > 0"
    @open="open"
    @back="emit('update', goBack(data))"
    @forward="emit('update', goForward(data))"
  />
  <div
    v-else
    class="flex flex-1 select-none flex-col items-center justify-center gap-1 text-[13px] text-fg-faint"
  >
    <span>File</span>
    <span class="max-w-full truncate px-4 font-mono text-[11px]">{{ data.path }}</span>
  </div>
</template>

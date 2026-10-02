<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: whether the view shows them rendered or as source holds
// across files and conversations for the page's lifetime.
const mode = ref<'preview' | 'source'>('preview')
</script>

<script setup lang="ts">
import { computed } from 'vue'
import { FileView, joinPath, relativePath, treeLayout, type ConversationFileService } from '@demicodes/plugin-sdk'
import { goBack, goForward, showFile, type FileData } from './data'

/**
 * The File view of one conversation (`file-previews.md`): one file of its
 * Host beside the Host's tree, over the conversation's files service. A file
 * picked in the tree or a link in a Markdown preview replaces the file, and
 * Back returns to it.
 */
const props = defineProps<{
  data: FileData
  files: ConversationFileService
}>()
const emit = defineEmits<{ update: [data: FileData] }>()

const workspace = computed(() => props.files.workspace)

function absolute(root: string, path: string): string {
  return path.startsWith('/') ? path : joinPath(root, path)
}

function open(root: string, path: string): void {
  emit('update', showFile(props.data, relativePath(root, path)))
}
</script>

<template>
  <FileView
    v-if="workspace"
    v-model:tree="treeLayout.open.value"
    v-model:tree-width="treeLayout.width.value"
    v-model:mode="mode"
    :source="workspace.source"
    :root="workspace.root"
    :root-name="workspace.name"
    :path="data.path ? absolute(workspace.root, data.path) : null"
    :can-back="data.back.length > 0"
    :can-forward="data.forward.length > 0"
    @open="open(workspace.root, $event)"
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

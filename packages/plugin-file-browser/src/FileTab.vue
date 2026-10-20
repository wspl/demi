<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: whether the view shows them rendered or as source holds
// across files and conversations for the page's lifetime.
const mode = ref<'preview' | 'source'>('preview')
</script>

<script setup lang="ts">
import { computed, watch } from 'vue'
import { FileView, treeLayout, usePage } from '@demicodes/plugin-sdk'
import { goBack, goForward, shownPath, showFile, treeShown, type FileData } from './data'
import { t } from './locales/text'

/**
 * The File view of one conversation: one file of its Host beside the Host's
 * tree, over the conversation's files service. A file picked in the tree or a
 * link in a Markdown preview replaces the file, and Back returns to it.
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

/** The directory the view was opened for, whose tree it shows: the conversation's unless a child's link opened it. */
const root = computed(() => props.data.root ?? workspace.value?.root ?? '/')

/** A file picked in the tree or a link in a document, in the directory the view was opened for. */
function open(path: string): void {
  emit('update', showFile(props.data, path, undefined, props.data.root))
}

const view = ref<InstanceType<typeof FileView> | null>(null)
// Show in Files: once the view shows the file, its tree shows it too. After the render, so the view has the file.
watch([() => props.data.tree, view], ([tree, shown]) => {
  if (!tree || !shown) {
    return
  }
  emit('update', treeShown(props.data))
  void shown.showInTree()
}, { immediate: true, flush: 'post' })
</script>

<template>
  <FileView
    v-if="workspace"
    ref="view"
    v-model:tree="treeLayout.open.value"
    v-model:tree-width="treeLayout.width.value"
    v-model:mode="mode"
    :source="workspace.source"
    :root="root"
    :root-name="data.root === undefined ? workspace.name : undefined"
    :path="data.path ? shownPath(root, data.path) : null"
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
    <span>{{ t('kind.title') }}</span>
    <span class="max-w-full truncate px-4 font-mono text-[11px]">{{ data.path }}</span>
  </div>
</template>

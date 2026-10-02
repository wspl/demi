<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: whether the view shows their text diff or both sides
// rendered holds across files and conversations for the page's lifetime.
const presentation = ref<'diff' | 'preview'>('diff')
</script>

<script setup lang="ts">
import { computed } from 'vue'
import {
  ChangeView,
  callChangeSource,
  joinPath,
  treeLayout,
  type ChangeSources,
  type ConversationFileService,
  type IntentService,
} from '@demicodes/plugin-sdk'
import { changePath, goBack, goForward, showChange, type ChangeData } from './data'

/**
 * The Change view of one conversation (`file-previews.md` § Changes): the
 * working tree's changes in Uncommitted, a call's retained edit in
 * Conversation, over the conversation's files service.
 */
const props = defineProps<{
  data: ChangeData
  files: ConversationFileService
  intents: IntentService
}>()
const emit = defineEmits<{ update: [data: ChangeData] }>()

const changes = computed<ChangeSources>(() => ({
  uncommitted: props.files.changes,
  conversation: props.data.call ? callChangeSource(props.data.call, props.files.readCallChange) : null,
}))
const selected = computed(() => changePath(props.data, props.data.mode, changes.value.uncommitted.files))
const root = computed(() => props.files.workspace?.root ?? props.files.root ?? '/')

/** A changed file opens in whatever shows files, by its absolute path. */
function open(path: string): void {
  props.intents.open('file', { path: path.startsWith('/') ? path : joinPath(root.value, path) })
}
</script>

<template>
  <ChangeView
    v-model:tree="treeLayout.open.value"
    v-model:tree-width="treeLayout.width.value"
    v-model:presentation="presentation"
    :mode="data.mode"
    :selected="selected"
    :changes="changes"
    :edit="data.edit"
    :root="root"
    :root-name="files.workspace?.name"
    :contents="files.workspace?.source.contents"
    :can-back="data.back.length > 0"
    :can-forward="data.forward.length > 0"
    :opens="intents.canOpen('file')"
    @update:mode="emit('update', showChange(data, $event, changePath(data, $event)))"
    @update:edit="emit('update', showChange(data, data.mode, selected, { call: data.call, edit: $event }))"
    @update:selected="emit('update', showChange(data, data.mode, $event))"
    @back="emit('update', goBack(data))"
    @forward="emit('update', goForward(data))"
    @open="open"
  />
</template>

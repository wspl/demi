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
  usePage,
  type ChangeSources,
} from '@demicodes/plugin-sdk'
import { changePath, goBack, goForward, showChange, type ChangeData } from './data'

/**
 * The Change view of one conversation (`file-previews.md` § Changes): the
 * working tree's changes in Uncommitted, a call's retained edit in
 * Conversation, over the conversation's files service.
 */
const props = defineProps<{
  conversation: string
  tabId: string
  data: ChangeData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: ChangeData] }>()

const page = usePage()
const files = page.files(props.conversation)
files.showChanges()
const intents = page.intents

const changes = computed<ChangeSources>(() => ({
  uncommitted: files.changes,
  conversation: props.data.call ? callChangeSource(props.data.call, files.edit) : null,
}))
const selected = computed(() => changePath(props.data, props.data.mode, changes.value.uncommitted.files))
const root = computed(() => files.workspace?.root ?? files.root ?? '/')

/** A changed file opens in whatever shows files, by its absolute path. */
function open(path: string): void {
  const absolute = path.startsWith('/') ? path : joinPath(root.value, path)
  intents.open(props.conversation, { intent: 'file', payload: { path: absolute } })
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

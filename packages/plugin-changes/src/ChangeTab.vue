<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: whether the view shows their text diff or both sides
// rendered holds across files and conversations for the page's lifetime.
const presentation = ref<'diff' | 'preview'>('diff')
</script>

<script setup lang="ts">
import { computed, watch } from 'vue'
import { ChangeView, joinPath, treeLayout, usePage } from '@demicodes/plugin-sdk'
import {
  changePath,
  goBack,
  goForward,
  holdShownChange,
  placeBranch,
  showBranch,
  showChange,
  showEdit,
  showItem,
  viewDirectory,
  type ChangeData,
} from './data'
import { useChangeSources } from './sources'

/**
 * The Change view of one conversation (`file-previews.md` § Changes): the
 * repository's commits and uncommitted changes in Commits, a request's in
 * Conversation, over
 * the conversation's files service, which reads the request from the
 * transcript as it is now, so the view follows calls that end later.
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
files.showChanges(() => viewDirectory(props.data))
const intents = page.intents

const changes = useChangeSources(files, () => props.data)
// Each frame of a turn replaces the product state the answer is read from:
// the view is given it as a value, which changes only with the answer.
const opens = computed(() => intents.canOpen('file'))
const selected = computed(() => changePath(props.data, props.data.mode, changes.value.uncommitted.files))
// The view shows the directory it was opened for: the one its agent worked in, the conversation's without one.
const viewRoot = computed(() => viewDirectory(props.data))
const root = computed(() => viewRoot.value ?? files.workspace?.root ?? files.root ?? '/')

// The first listed file, shown while the user has picked none, stays shown:
// a file the agent or a build changes later may list before it.
watch(selected, () => {
  const held = holdShownChange(props.data, changes.value.uncommitted.files)
  if (held !== props.data)
    emit('update', held)
}, { immediate: true })

/** A changed file opens in whatever shows files, by its absolute path. */
function open(path: string): void {
  const absolute = path.startsWith('/') ? path : joinPath(root.value, path)
  intents.open(props.conversation, { intent: 'file', payload: viewRoot.value === undefined ? { path: absolute } : { path: absolute, root: viewRoot.value } })
}
</script>

<template>
  <ChangeView
    v-model:tree="treeLayout.open.value"
    v-model:tree-width="treeLayout.width.value"
    v-model:commits-height="treeLayout.commitsHeight.value"
    v-model:presentation="presentation"
    :mode="data.mode"
    :selected="selected"
    :commit="data.commit"
    :branch="data.branch"
    :place="data.place"
    :changes="changes"
    :edit="data.request?.edit ?? null"
    :root="root"
    :root-name="viewRoot === undefined ? files.workspace?.name : undefined"
    :contents="files.workspace?.source.contents"
    :can-back="data.back.length > 0"
    :can-forward="data.forward.length > 0"
    :opens="opens"
    @update:mode="emit('update', showChange(data, $event, changePath(data, $event)))"
    @update:edit="emit('update', showEdit(data, $event))"
    @update:selected="emit('update', showChange(data, data.mode, $event))"
    @update:commit="emit('update', showItem(data, $event))"
    @update:branch="emit('update', showBranch(data, $event))"
    @placed="emit('update', placeBranch(data, $event))"
    @back="emit('update', goBack(data))"
    @forward="emit('update', goForward(data))"
    @open="open"
  />
</template>

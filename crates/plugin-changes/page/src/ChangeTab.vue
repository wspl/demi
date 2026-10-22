<script setup lang="ts">
import { computed, watch } from 'vue'
import { ChangeView, joinPath, treeLayout, usePage, usePaneReady, type TabUpdate } from '@demicodes/plugin-sdk'
import {
  changePath,
  changeView,
  holdShownChange,
  placeBranch,
  showBranch,
  showChange,
  showEdit,
  showItem,
  showPresentation,
  viewDirectory,
  type ChangeData,
} from './data'
import { useChangeSources } from './sources'

/**
 * The Change view of one conversation: the repository's commits and uncommitted
 * changes in Commits, a request's in Conversation, over the conversation's
 * files service, which reads the request from the transcript as it is now, so
 * the view follows calls that end later. Each move of the view is a step of
 * the web browser's history, which its Back and Forward take while they stay
 * in the conversation's panel.
 */
const props = defineProps<{
  conversation: string
  tabId: string
  data: ChangeData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: ChangeData, how?: TabUpdate] }>()

const page = usePage()
const files = page.files(props.conversation)
// What the view lists is the working tree of the conversation's directory: the pane
// reads until the conversation's opening brings it.
usePaneReady(() => files.workspace !== null)
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
    emit('update', held, { replace: true })
}, { immediate: true })

/** Diff or Preview, chosen here: the user's choice from now on, and this change's in the address. */
function choose(chosen: 'diff' | 'preview'): void {
  changeView.value = chosen
  emit('update', showPresentation(props.data, chosen), { replace: true })
}

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
    :presentation="data.view ?? changeView"
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
    :can-back="page.panel.canGo(conversation, 'back')"
    :can-forward="page.panel.canGo(conversation, 'forward')"
    :opens="opens"
    @update:mode="emit('update', showChange(data, $event, changePath(data, $event)))"
    @update:edit="emit('update', showEdit(data, $event))"
    @update:selected="emit('update', showChange(data, data.mode, $event))"
    @update:commit="emit('update', showItem(data, $event))"
    @update:presentation="choose"
    @update:branch="emit('update', showBranch(data, $event), { replace: true })"
    @placed="emit('update', placeBranch(data, $event), { replace: true })"
    @back="page.panel.go('back')"
    @forward="page.panel.go('forward')"
    @open="open"
  />
</template>

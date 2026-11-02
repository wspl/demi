<script setup lang="ts">
import { computed, watch } from 'vue'
import { ChangeView, baseName, joinPath, treeLayout, usePage, usePaneReady, type RepositoryEntry, type TabUpdate } from '@demicodes/plugin-sdk'
import {
  changePath,
  changeView,
  holdShownChange,
  keepRepository,
  placeBranch,
  showBranch,
  showChange,
  showEdit,
  showItem,
  showPresentation,
  showRepository,
  viewDirectory,
  type ChangeData,
} from './data'
import { useShownDirectory } from './shown'
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
// The view shows the directory it was opened for, or the one picked: else in
// Commits the repository the agents changed a file in last.
const { repositories, directory: viewRoot } = useShownDirectory(files, () => props.data)
// What the view lists is the working tree of the directory it shows: the pane
// reads until the conversation's opening brings it, and until the list of
// repositories says which one that is.
usePaneReady(() => files.tree !== null && viewRoot.value !== null)
files.showChanges(() => viewRoot.value)
const intents = page.intents

const changes = useChangeSources(files, () => props.data, () => viewRoot.value)
// Each frame of a turn replaces the product state the answer is read from:
// the view is given it as a value, which changes only with the answer.
const opens = computed(() => intents.canOpen('file'))
const selected = computed(() => changePath(props.data, props.data.mode, changes.value.uncommitted.files))
const root = computed(() => viewRoot.value ?? files.tree?.root ?? files.root ?? '/')
// The menu lists what was found, and the directory shown when a link opened
// one outside it; it shows while there is a repository to choose, its room
// kept while the list is on its way.
const menu = computed<readonly RepositoryEntry[] | undefined>(() => {
  if (viewRoot.value === null)
    return []
  const list = repositories.value?.list ?? []
  const named = viewRoot.value
  if (list.length === 0 && named === undefined)
    return undefined
  const shown = named ?? files.root
  return shown === null || list.some((entry) => entry.root === shown) ? list : [...list, { root: shown, name: baseName(shown) }]
})
/** The repository shown, null for the conversation's directory. */
const shownRepository = computed(() => viewRoot.value ?? null)

/** The repository the user picked stays shown; picking the one shown keeps it, as it is. */
function pickRepository(picked: string | null): void {
  if (picked === null)
    return
  const root = picked === files.root ? null : picked
  const next = root === shownRepository.value ? keepRepository(props.data, root) : showRepository(props.data, root)
  if (next !== props.data)
    emit('update', next, { replace: true })
}

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

/**
 * The directory the agent that named the view's files worked in: a request's,
 * or a link's; a repository picked from the menu, or shown by default, is no
 * agent's.
 */
const agentRoot = computed(() => {
  const named = viewDirectory(props.data)
  if (named === undefined || props.data.root === undefined)
    return named
  return repositories.value?.list.some((entry) => entry.root === named) ? undefined : named
})

/** A changed file opens in whatever shows files, by its absolute path, under the directory its agent worked in. */
function open(path: string): void {
  const absolute = path.startsWith('/') ? path : joinPath(root.value, path)
  const agent = agentRoot.value
  intents.open(props.conversation, { intent: 'file', payload: agent === undefined ? { path: absolute } : { path: absolute, root: agent } })
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
    :repositories="menu"
    :repository="viewRoot ?? files.root"
    @update:repository="pickRepository"
    :changes="changes"
    :edit="data.request?.edit ?? null"
    :root="root"
    :root-name="viewRoot === undefined ? files.tree?.name : undefined"
    :contents="files.tree?.source.contents"
    :can-back="page.panel.canGo(conversation, 'back')"
    :can-forward="page.panel.canGo(conversation, 'forward')"
    :opens="opens"
    @update:mode="emit('update', showChange(data, $event, changePath(data, $event)))"
    @update:edit="emit('update', showEdit(data, $event))"
    @update:selected="emit('update', showChange(data, data.mode, $event))"
    @update:commit="emit('update', showItem(keepRepository(data, shownRepository), $event))"
    @update:presentation="choose"
    @update:branch="emit('update', showBranch(keepRepository(data, shownRepository), $event), { replace: true })"
    @placed="emit('update', placeBranch(data, $event), { replace: true })"
    @back="page.panel.go('back')"
    @forward="page.panel.go('forward')"
    @open="open"
  />
</template>

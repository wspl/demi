<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch } from 'vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import type { ChangeMode } from '@demicodes/web-ui/files/changes'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { fileSource } from '../api/files'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { useConversations } from './store'
import { useWorkPanel } from './work'

/**
 * The work panel beside one conversation: its tabs from the work store, and
 * the workspace from the conversation's execution target, the Host's
 * files and its working tree. The Change tab's Uncommitted list follows the
 * conversation: it is taken when the tab shows, listed again after each of
 * the conversation's tool calls finishes while it shows (marked stale
 * otherwise, for the next showing), and when the page becomes visible
 * again. Conversation diffs read retained contents independently of the live device.
 */
const props = defineProps<{ conversationId: string }>()
const emit = defineEmits<{ close: [] }>()

const conversations = useConversations()
const resources = useResources()
const work = useWorkPanel()

const state = computed(() => work.stateFor(props.conversationId))
const conversation = computed(
  () => conversations.items.find((item) => item.id === props.conversationId) ?? null,
)
const workspace = computed(() => {
  if (!conversation.value) {
    return undefined
  }
  const execution = executionFor(conversation.value)
  const device = resources.deviceById(execution.deviceId)
  if (!device || !execution.path) {
    return undefined
  }
  return {
    source: fileSource({
      directory: `/conversations/${encodeURIComponent(props.conversationId)}/fs`,
      text: `/conversations/${encodeURIComponent(props.conversationId)}/fs/file`,
    }, device),
    root: execution.path,
    // The Cloud's own session directory has no name worth showing; it is the workspace.
    name: execution.directory === null ? 'Workspace' : undefined,
    changes: state.value.changes,
  }
})

/** Whether the Uncommitted list is on screen: the active tab is the change tab in that mode. */
const uncommittedShown = computed(() => {
  const active = state.value.tabs.find((tab) => tab.id === state.value.activeId)
  return active?.kind === 'change' && active.mode === 'uncommitted'
})
watch(
  uncommittedShown,
  (shown) => {
    if (shown) {
      state.value.changes.ensureFresh()
    }
  },
  { immediate: true },
)

/** The conversation's tool calls that have finished; each may have changed files. */
const finishedToolCalls = computed(
  () =>
    conversation.value?.blocks.filter(
      (block) => block.type === 'tool_call' && block.status !== 'executing',
    ).length ?? 0,
)
watch(finishedToolCalls, () => {
  if (uncommittedShown.value) {
    state.value.changes.refresh()
  } else {
    state.value.changes.markStale()
  }
})

function refreshVisible(): void {
  if (document.visibilityState === 'visible' && uncommittedShown.value) {
    state.value.changes.refresh()
  }
}
onMounted(() => {
  document.addEventListener('visibilitychange', refreshVisible)
})
onBeforeUnmount(() => {
  document.removeEventListener('visibilitychange', refreshVisible)
})

/**
 * A new file tab shows the file of the active file tab again, else the
 * first file at the workspace root; with neither there is nothing to open.
 */
async function addFile(): Promise<void> {
  const current = state.value
  const active = current.tabs.find((tab) => tab.id === current.activeId)
  const fileTab = active?.kind === 'file' ? active : current.tabs.find((tab) => tab.kind === 'file')
  if (fileTab?.kind === 'file') {
    work.addFile(current, fileTab.path)
    return
  }
  const source = workspace.value
  if (!source) {
    return
  }
  try {
    const entries = await source.source.list(source.root)
    const first = entries.find((entry) => !entry.isDirectory)
    if (first) {
      work.addFile(current, first.name)
    }
  } catch (error) {
    reportError('Could not list the workspace', error, { userVisible: true })
  }
}

function add(kind: 'file' | 'change', mode?: ChangeMode): void {
  if (kind === 'file') {
    void addFile()
  } else {
    work.addChange(state.value, mode ?? 'uncommitted')
  }
}
</script>

<template>
  <WorkPanel
    :tabs="state.tabs"
    :active-id="state.activeId"
    :workspace="workspace"
    :read-call-change="state.readCallChange"
    :history-root="conversation ? executionFor(conversation).path ?? undefined : undefined"
    @select="work.select(state, $event)"
    @close-tabs="work.close(state, $event)"
    @add="add"
    @show-change="(id, mode, path, selection) => work.showChange(state, id, mode, path, selection)"
    @open="work.open(state, $event)"
    @back="work.back(state, $event)"
    @forward="work.forward(state, $event)"
    @close="emit('close')"
  />
</template>

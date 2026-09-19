<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, watch } from 'vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import { hostWorkTabId, withHostTabs } from '@demicodes/web-ui/agent/work-panel'
import { useLiveSession } from '@demicodes/web-ui/browser/useLiveSession'
import { reportActivity } from '../api/activity'
import { conversationFileRoutes, fileSource } from '../api/files'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { useConversations } from './store'
import { useWorkPanel } from './work'

/**
 * The work panel beside one conversation: its tabs from the work store, the
 * conversation browser's own tabs from its live view, and
 * the workspace from the conversation's execution target, the Host's
 * files and its working tree. The fixed Change summary follows the conversation:
 * refresh on panel opening, completed tool calls, and page visibility. Conversation diffs read retained contents independently of
 * the live device.
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
    source: fileSource(conversationFileRoutes(props.conversationId), device),
    root: execution.path,
    // The Cloud's own session directory has no name worth showing; it is the workspace.
    name: execution.directory === null ? 'Workspace' : undefined,
    changes: state.value.changes,
  }
})

/**
 * The live view of the conversation's browser, while the panel is open
 * (`browser-live-view.md` § Opening a view). A closed panel watches nothing,
 * so the Host captures nothing.
 */
const streamUrl = computed(() => {
  if (!state.value.open) {
    return null
  }
  const url = new URL(
    `/api/conversations/${encodeURIComponent(props.conversationId)}/streams/browser`,
    window.location.href,
  )
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  return url.toString()
})
const live = useLiveSession(streamUrl, {
  onOperation: () => void reportActivity(props.conversationId),
})
const tabs = computed(() => withHostTabs(state.value.tabs, live.value?.state.tabs ?? []))

// A Host tab that closed leaves the panel showing the next one, or the Change.
watch(tabs, (current) => {
  const active = state.value.activeId
  if (active?.startsWith('host:') && !current.some((tab) => tab.id === active)) {
    const first = live.value?.state.tabs[0]
    work.select(state.value, first ? hostWorkTabId(first.id) : 'change')
  }
})

// The fixed Change summary remains visible across all sections.
watch(state, (current) => current.changes.refresh(), { immediate: true })

/** The conversation's tool calls that have finished; each may have changed files. */
const finishedToolCalls = computed(
  () =>
    conversation.value?.blocks.filter(
      (block) => block.type === 'tool_call' && block.status !== 'executing',
    ).length ?? 0,
)
watch(finishedToolCalls, () => {
  state.value.changes.refresh()
})

function refreshVisible(): void {
  if (document.visibilityState === 'visible') {
    state.value.changes.refresh()
  }
}
onMounted(() => {
  document.addEventListener('visibilitychange', refreshVisible)
})
onBeforeUnmount(() => {
  document.removeEventListener('visibilitychange', refreshVisible)
})
</script>

<template>
  <WorkPanel
    :tabs="tabs"
    :active-id="state.activeId"
    :live="live ?? undefined"
    :workspace="workspace"
    :read-call-change="state.readCallChange"
    :history-root="conversation ? executionFor(conversation).path ?? undefined : undefined"
    @select="work.select(state, $event)"
    @add-browser="work.addBrowser(state)"
    @close-tabs="work.closeTabs(state, $event)"
    @update-browser="work.updateBrowser(state, $event)"
    @show-change="(id, mode, path, selection) => work.showChange(state, id, mode, path, selection)"
    @open="work.open(state, $event)"
    @back="work.back(state, $event)"
    @forward="work.forward(state, $event)"
    @close="emit('close')"
  />
</template>

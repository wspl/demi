<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import type { PanelTabKind } from '@demicodes/web-ui/agent/panel-kinds/kind'
import { usePluginHost } from '@demicodes/web-ui/plugins/client'
import { pluginPanelKinds, type PanelKinds } from '@demicodes/web-ui/plugins/slots'
import { PLUGIN_PAGES } from '../plugins/pages'
import { pluginEnabled } from '../plugins/host'
import { useProduct } from '../state/product'
import { conversationFileRoutes, fileSource } from '../api/files'
import { useResources } from '../state/resources'
import { executionFor } from '../targets/execution'
import { readEditCopies } from './changes'
import { useConversations } from './store'
import { useWorkPanel } from './work'

/**
 * The work panel beside one conversation: its saved tabs and fixed views from
 * the work store, the tab kinds it can show, and
 * the workspace from the conversation's execution target, the Host's
 * files and its working tree. The fixed Change summary follows the conversation:
 * refresh on panel opening, completed tool calls, and page visibility. Conversation diffs read retained contents independently of
 * the live device.
 */
const props = defineProps<{ conversationId: string }>()
const emit = defineEmits<{ close: [] }>()

const conversations = useConversations()
const resources = useResources()
const product = useProduct()
const host = usePluginHost()
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
 * The plugins' tab kinds for this conversation (`plugins.md` § The page),
 * for as long as the panel is open beside it and the user has each plugin
 * on. A closed panel reads no tab list and holds no view.
 */
const plugins = shallowRef<Required<PanelKinds> | null>(null)
watch(
  () =>
    [
      props.conversationId,
      state.value.open,
      PLUGIN_PAGES.map((page) => pluginEnabled(product.snapshot, page.plugin)).join(),
    ] as const,
  ([conversationId, open]) => {
    plugins.value?.dispose()
    plugins.value = null
    if (!open) {
      return
    }
    void work.load(conversationId)
    plugins.value = pluginPanelKinds(
      PLUGIN_PAGES,
      host,
      (plugin) => pluginEnabled(product.snapshot, plugin),
      {
        conversation: conversationId,
        tabs: {
          bound: (kind) =>
            work.stateFor(conversationId).panel.tabs.filter((tab) => tab.kind === kind).map((tab) => tab.data),
          add: (kind, data) => void work.add(conversationId, kind, data, { select: false }),
        },
      },
    )
  },
  { immediate: true },
)

const kinds = computed<PanelTabKind[]>(() => plugins.value?.kinds ?? [])

/** Closing a tab removes it at once; its kind then does what a closed tab of it needs. */
function closeTabs(ids: string[]): void {
  const closing = state.value.panel.tabs.filter((tab) => ids.includes(tab.id))
  work.closeTabs(props.conversationId, ids)
  for (const tab of closing) {
    const kind = kinds.value.find((candidate) => candidate.kind === tab.kind)
    const parsed = kind?.schema.safeParse(tab.data)
    if (kind?.removed && parsed?.success) {
      kind.removed(parsed.data)
    }
  }
}

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
  // The agent may have changed what a plugin's tabs show, as the
  // conversation browser's tabs.
  plugins.value?.refresh()
})

// A plugin's kinds read what they show themselves when the page is shown again.
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
  plugins.value?.dispose()
})
</script>

<template>
  <WorkPanel
    :views="state.views"
    :panel="state.panel"
    :kinds="kinds"
    :workspace="workspace"
    :read-call-change="readEditCopies"
    :history-root="conversation ? executionFor(conversation).path ?? undefined : undefined"
    @select="work.select(conversationId, $event)"
    @add-tab="(kind, data) => work.add(conversationId, kind, data)"
    @update-tab="(id, data) => work.update(conversationId, id, data)"
    @close-tabs="closeTabs"
    @show-change="(id, mode, path, selection) => work.showChange(state, id, mode, path, selection)"
    @open="work.open(conversationId, $event)"
    @back="work.back(state, $event)"
    @forward="work.forward(state, $event)"
    @close="emit('close')"
  />
</template>

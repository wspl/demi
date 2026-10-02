<script setup lang="ts">
import { computed, onBeforeUnmount, shallowRef, watch } from 'vue'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import type { PanelTabKind } from '@demicodes/web-ui/agent/panel-kinds/kind'
import { usePluginHost } from '@demicodes/web-ui/plugins/client'
import { pluginPanelKinds, type ConversationFileService, type PanelKinds } from '@demicodes/web-ui/plugins/slots'
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
 * The work panel beside one conversation: its saved tabs and pinned tabs from
 * the work store, and the kinds the plugin pages make for it over the
 * conversation's files service and its intents. After each finished tool
 * call the kinds read again what the agent may have changed.
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
/** Where the conversation's work runs. */
const execution = computed(() => (conversation.value ? executionFor(conversation.value) : null))
/** The Host's tree and contents, while the conversation's device and directory are known. */
const workspace = computed(() => {
  const target = execution.value
  const device = target ? resources.deviceById(target.deviceId) : null
  if (!target || !device || !target.path) {
    return null
  }
  return {
    source: fileSource(conversationFileRoutes(props.conversationId), device),
    root: target.path,
    // The Cloud's own session directory has no name worth showing; it is the workspace.
    name: target.directory === null ? 'Workspace' : undefined,
  }
})

/**
 * The conversation's files as the kinds read them (`plugin-pages.md`
 * § Services). Each field is read when a kind reads it, so it follows the
 * conversation's Host.
 */
const files: ConversationFileService = {
  get workspace() {
    return workspace.value
  },
  get root() {
    return execution.value?.path ?? null
  },
  get changes() {
    return state.value.changes
  },
  readCallChange: readEditCopies,
}

/**
 * The plugins' tab kinds for this conversation (`plugin-pages.md` § Work panel kinds),
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
        files,
        intents: {
          open: (intent, payload) => work.openIn(conversationId, intent, payload),
          canOpen: (intent) => work.canOpen(intent),
        },
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

/** The conversation's tool calls that have finished; each may have changed files. */
const finishedToolCalls = computed(
  () =>
    conversation.value?.blocks.filter(
      (block) => block.type === 'tool_call' && block.status !== 'executing',
    ).length ?? 0,
)
// The agent may have changed what a kind shows, as the working tree or the
// conversation browser's tabs; a kind reads again itself when the page is
// shown again.
watch(finishedToolCalls, () => {
  plugins.value?.refresh()
})
onBeforeUnmount(() => {
  plugins.value?.dispose()
})
</script>

<template>
  <WorkPanel
    :panel="state.panel"
    :pinned="state.pinned"
    :kinds="kinds"
    @select="work.select(conversationId, $event)"
    @add-tab="(kind, data) => work.add(conversationId, kind, data)"
    @update-tab="(id, data) => work.update(conversationId, id, data)"
    @update-pinned="(kind, data) => work.updatePinned(conversationId, kind, data)"
    @close-tabs="closeTabs"
    @close="emit('close')"
  />
</template>

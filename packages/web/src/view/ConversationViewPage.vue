<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from 'vue'
import ConversationView from '@demicodes/web-ui/view/ConversationView.vue'
import MediaViewer from '@demicodes/web-ui/files/MediaViewer.vue'
import { provideBlobUrl } from '@demicodes/web-ui/agent/media-source'
import { provideCostWatch } from '@demicodes/web-ui/agent/context-cost'
import { provideHistoryReading } from '@demicodes/web-ui/agent/history-reading'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { providePageHost } from '@demicodes/web-ui/plugins/page'
import { pageTitle } from '@demicodes/web-ui/sidebar/tab-attention'
import { loadRegionRenderers } from '@demicodes/web-ui/infra/region-load'
import { apiUrl } from '../api/client'
import type { ViewedConversation } from '../api/generated/web-api'
import { BLOCK_PAGES } from '../plugins/generated/blocks'
import { accountlessPageHost } from '../plugins/accountless-host'
import { tellParent } from './parent'
import { viewPath, watchViewed } from './conversation'

/**
 * A view link's page of a conversation: what the conversation's page shows,
 * live, for someone who only watches, read and followed through the link's
 * routes (`crates/backend-http/src/conversation_views.rs`). It tells the
 * window that frames it once the transcript shows; `lost` is told when the
 * connection could not be made, for the page to read whether the link still
 * opens, and `retry` when the reader asks to load a failed opening again,
 * which the page does whole.
 */
const props = defineProps<{ token: string; viewed: ViewedConversation }>()
const emit = defineEmits<{ lost: []; retry: [] }>()

const base = viewPath(props.token)
provideBlobUrl((ref, mediaType) => apiUrl(`${base}/blobs/${encodeURIComponent(ref)}?${new URLSearchParams({ type: mediaType })}`))
providePageHost(accountlessPageHost('A view link’s page', {
  pages: () => BLOCK_PAGES,
  blockDocument: (plugin, tag, id) =>
    `/api${base}/plugins/${encodeURIComponent(plugin)}/blocks/${encodeURIComponent(tag)}/${encodeURIComponent(id)}`,
}))
const mediaViewer = provideMediaViewer()

const viewed = watchViewed(props.token, props.viewed, () => emit('lost'))
onBeforeUnmount(() => viewed.dispose())
const conversation = viewed.conversation
provideHistoryReading((node) => viewed.history.reading(conversation, node))
provideCostWatch((window) => viewed.watchRecentCost(window))

const schedules = computed(() => conversation.metadata.shown.value?.schedules ?? [])
const output = computed(() => conversation.requestOutputs[conversation.id] ?? null)
function agentOutput(id: string) {
  return conversation.requestOutputs[id] ?? null
}
async function readBlock(node: string | null, id: string): Promise<void> {
  await viewed.history.readWhole(conversation, node, id)
}
function shown(node: string | null, block: string | null, within: number): void {
  viewed.history.showAt(conversation, node, block, within)
}

// Once the transcript shows, the window that frames the page hears so, and
// what its regions need loads in the background.
watch(() => conversation.history.windows.length > 0, (showing) => {
  if (showing) {
    tellParent({ type: 'demi.view', state: 'live' })
    loadRegionRenderers()
  }
}, { immediate: true, flush: 'post' })
watch(() => conversation.title, (title) => {
  document.title = pageTitle(title?.trim() || 'Demi')
}, { immediate: true })
</script>

<template>
  <ConversationView
    :conversation="conversation"
    :run="viewed.run.value"
    :usage="conversation.contextUsage"
    :schedules="schedules"
    :cost="viewed.cost()"
    :agent-cost="(id) => viewed.cost(id)"
    :output="output"
    :agent-output="agentOutput"
    :read-block="readBlock"
    subagents-streamed
    @read-before="viewed.history.readBefore(conversation, null)"
    @read-after="viewed.history.readAfter(conversation, null)"
    @read-subagent="(id) => viewed.history.readBefore(conversation, id)"
    @read-subagent-after="(id) => viewed.history.readAfter(conversation, id)"
    @shown="shown"
    @show-subagent="viewed.showSubagent"
    @retry-load="emit('retry')"
  />
  <MediaViewer :viewer="mediaViewer" />
</template>

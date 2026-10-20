<script setup lang="ts">
import { reactive, ref } from 'vue'
import DockWindowArea from '@demicodes/web-ui/agent/DockWindowArea.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import { dockWindowShare } from '../dock-window-share'
import { provideEditSelection } from '@demicodes/web-ui/agent/edit-selection'
import { provideMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { useAgentContext } from '../agent-context'
import { galleryConversationFiles } from '../fixtures/message-files'
import { SLOT_NODE, slotBlocks } from '../fixtures/request-changes'
import { slotSubagent } from '../fixtures/subagents'
import { useGalleryTranscripts } from '../fixtures/transcripts'
import { galleryBrowser } from '../fixtures/live-browser'
import { galleryFiles, useGalleryWork } from '../fixtures/work-panel'
import { createGalleryWorkspace } from '../fixtures/workspace'
import GalleryWorkPanel from './GalleryWorkPanel.vue'

/**
 * The Agents window over a child that works in another checkout of the
 * project (`crates/agent-server/src/node/mod.rs`), beside the conversation's
 * work panel: the child's links, its file change and the folder in its
 * status bar open the panel for the child's directory, while the panel's
 * own tabs stay on the conversation's. The agent's conversation is in
 * `/Users/alex/Projects/shop`.
 */
const agentContext = useAgentContext()
const workspace = createGalleryWorkspace()
const files = galleryFiles(workspace)
const work = useGalleryWork('file', { files, browser: galleryBrowser([]) })
useGalleryTranscripts(() => ({ blocks: [], subagents: [{ id: SLOT_NODE, blocks: slotBlocks() }] }))
provideEditSelection(() => work.selectEdit)
// A link in the child's messages opens the panel for the directory the child wrote it in.
const conversationFiles = galleryConversationFiles(workspace.source, ({ path, line, root }) => {
  work.openIn({ intent: 'file', payload: { path, ...(line === null ? {} : { line }), ...(root === undefined ? {} : { root }) } })
}, ({ commit, root }) => {
  work.openIn({ intent: 'commit', payload: { commit, ...(root === undefined ? {} : { root }) } })
}, ({ path, root }) => {
  work.openIn({ intent: 'file', payload: { path, tree: true, ...(root === undefined ? {} : { root }) } })
}, (id, { name, size, url }) => {
  work.openIn({ intent: 'attachment', payload: { id, name, size, url } })
})
provideMessageFiles(() => ({ ...conversationFiles, cwd: workspace.root }))

const agents = reactive([slotSubagent()])
const open = ref(true)
const activeId = ref<string | null>(agents[0]!.id)
/** Stop Agent or Stop All on the child: it stops, and its tab stays while the window shows it. */
function stop(): void {
  const agent = agents[0]!
  if (agent.phase === 'running') {
    agent.phase = 'aborted'
    agent.endedAt = new Date().toISOString()
  }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap gap-2">
      <Button variant="ghost" size="sm" :disabled="open" @click="open = true">Open Again</Button>
    </div>
    <DockWindowArea v-model:share="dockWindowShare" class="h-[26rem]">
      <SubagentPanel
        v-model:open="open"
        v-model:active-id="activeId"
        :agents="agents"
        @abort="stop"
        @abort-agent="stop"
        v-bind="agentContext.panel"
      />
    </DockWindowArea>
    <div class="h-[26rem] overflow-hidden rounded-lg border border-line">
      <GalleryWorkPanel :work="work" @close="work.open.value = false" />
    </div>
  </div>
</template>

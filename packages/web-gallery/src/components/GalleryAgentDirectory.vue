<script setup lang="ts">
import { reactive, ref } from 'vue'
import { provideEditSelection } from '@demicodes/web-ui/agent/edit-selection'
import { agentDirectories } from '@demicodes/web-ui/agent/working-directory'
import { provideMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { galleryConversationFiles } from '../fixtures/message-files'
import { SLOT_NODE, slotBlocks } from '../fixtures/request-changes'
import { slotSubagent } from '../fixtures/subagents'
import { useGalleryTranscripts } from '../fixtures/transcripts'
import { galleryBrowser } from '../fixtures/live-browser'
import { galleryFiles, useGalleryWork } from '../fixtures/work-panel'
import { createGalleryProject } from '../fixtures/project'
import GalleryOwnWindows from './GalleryOwnWindows.vue'
import GalleryWorkWindows from './GalleryWorkWindows.vue'

/**
 * The Agents window over a child that works in another checkout of the
 * project (`crates/agent-server/src/node/mod.rs`), beside the conversation's
 * work windows: the child's links, its file change and the folder in its
 * status bar open Files and Changes for the child's directory, while their
 * own tabs stay on the conversation's. The agent's conversation is in
 * `/Users/alex/Projects/shop`.
 */
const project = createGalleryProject()
/**
 * The branch checked out in the child's folder: Check Out Another Branch steps
 * through two branches and a detached HEAD, as the child's commands would.
 */
const BRANCHES: (string | null)[] = ['feat/cart', 'docs/guide', null]
const slotBranch = ref<string | null>(BRANCHES[0]!)
function checkOutNext(): void {
  slotBranch.value = BRANCHES[(BRANCHES.indexOf(slotBranch.value) + 1) % BRANCHES.length]!
}
const agents = reactive([slotSubagent()])
const files = galleryFiles(project, {
  directories: () => agentDirectories(project.root, agents),
  slotBranch: () => slotBranch.value,
})
const work = useGalleryWork('file', { files, browser: galleryBrowser([]) })
useGalleryTranscripts(() => ({ blocks: [], subagents: [{ id: SLOT_NODE, blocks: slotBlocks() }] }))
provideEditSelection(() => work.selectEdit)
// A link in the child's messages opens its window for the directory the child wrote it in.
const conversationFiles = galleryConversationFiles(project.source, ({ path, line, root }) => {
  work.openIn({ intent: 'file', payload: { path, ...(line === null ? {} : { line }), ...(root === undefined ? {} : { root }) } })
}, ({ commit, root }) => {
  work.openIn({ intent: 'commit', payload: { commit, ...(root === undefined ? {} : { root }) } })
}, ({ path, root }) => {
  work.openIn({ intent: 'file', payload: { path, tree: true, ...(root === undefined ? {} : { root }) } })
}, (id, { name, size, url }) => {
  work.openIn({ intent: 'attachment', payload: { id, name, size, url } })
}, (root) => files.history(root), ({ root, branch }) => {
  work.openIn({ intent: 'branch', payload: { root, branch } })
})
provideMessageFiles(() => ({ ...conversationFiles, cwd: project.root }))

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
      <Button variant="ghost" size="sm" @click="checkOutNext">Check Out Another Branch</Button>
    </div>
    <div class="gallery-frame h-[26rem] overflow-hidden">
      <GalleryOwnWindows
        v-model:open="open"
        v-model:active-id="activeId"
        :agents="agents"
        fill
        @abort="stop"
        @abort-agent="stop"
      />
    </div>
    <div class="h-[26rem] overflow-hidden rounded-lg border border-line">
      <GalleryWorkWindows :work="work" />
    </div>
  </div>
</template>

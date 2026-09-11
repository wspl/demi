<script setup lang="ts">
import { reactive, ref } from 'vue'
import ChatSession, {
  type ChatSessionState,
} from '@demicodes/web-ui/agent/ChatSession.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import SidebarAccount from '@demicodes/web-ui/sidebar/SidebarAccount.vue'
import WorkspaceDirectoryMenu from '@demicodes/web-ui/hosts/WorkspaceDirectoryMenu.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { transcriptDemoBlocks } from '../fixtures/blocks'
import { gallerySubagents } from '../fixtures/subagents'
import { galleryTerminals } from '../fixtures/terminals'
import { createGalleryFileHosts } from '../fixtures/files'
import GalleryComposer from './GalleryComposer.vue'

const props = withDefaults(defineProps<{ showActivity?: boolean }>(), { showActivity: true })
const session = reactive<ChatSessionState>({
  id: 'shared-product-session',
  title: 'Shared session',
  blocks: transcriptDemoBlocks(),
  queue: [],
  pendingSteers: [],
  phase: 'idle',
  load: 'ready',
  lastError: null,
  archived: false,
  status: 'idle',
  scroll: null,
  subagents: props.showActivity ? gallerySubagents() : [],
  terminals: props.showActivity ? galleryTerminals() : [],
})
const hosts = createGalleryFileHosts()
const folder = ref({
  deviceId: hosts[0]!.id,
  path: hosts[0]!.source.home,
})
async function selectFolder(deviceId: string, path: string): Promise<boolean> {
  folder.value = {
    deviceId,
    path,
  }
  return true
}
function abortAgents(): void {
  for (const agent of session.subagents) {
    if (agent.phase !== 'running') {
      continue
    }
    agent.phase = 'aborted'
    agent.endedAt = new Date().toISOString()
  }
}
</script>
<template>
  <div class="h-[36rem] overflow-hidden rounded-xl border border-border">
    <SidebarLayout label="Shared session">
      <template #sidebar>
        <div class="w-48 p-3">
          <SidebarAccount :account="{ name: '', email: 'new@example.com' }" />
        </div>
      </template>
      <ChatSession
        :conversation="session"
        has-provider
        @archive="session.archived = true"
        @save-scroll="(_id, state) => (session.scroll = state)"
        @abort-subagents="abortAgents"
      >
        <template #workspace>
          <WorkspaceDirectoryMenu
            :overlay-store="appOverlayStore"
            :path="folder.path"
            :device-id="folder.deviceId"
            :locked="session.archived"
            browse-enabled
            :recent-directories="[]"
            :hosts="hosts"
            :select-folder="selectFolder"
          />
        </template>
        <template #composer
          ><GalleryComposer
            placeholder="Ask Demi…"
            conversation-id="shared-product-session"
            :archived="session.archived"
            @restore="session.archived = false"
            :attachments="[{ name: 'ready-example.png', phase: 'ready' }]"
        /></template>
      </ChatSession>
    </SidebarLayout>
  </div>
</template>

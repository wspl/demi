<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import SidebarAccount from '@demicodes/web-ui/sidebar/SidebarAccount.vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import WorkspaceDirectoryMenu from '@demicodes/web-ui/hosts/WorkspaceDirectoryMenu.vue'
import type { HostDeviceOption, HostMenuMainHost } from '@demicodes/web-ui/hosts/types'
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
  pendingAction: null,
  archived: false,
  status: 'idle',
  scroll: null,
  subagents: props.showActivity ? gallerySubagents() : [],
  terminals: props.showActivity ? galleryTerminals() : [],
})
const hosts = createGalleryFileHosts()
const devices: HostDeviceOption[] = hosts.map((host) => ({
  id: host.id,
  name: host.label,
  online: host.online,
}))
const folder = ref({
  deviceId: hosts[0]!.id,
  path: hosts[0]!.source.home,
})
// Recent directories are the current host's Recent places, the way the product lists its projects there.
const recentDirectories = computed(() => {
  const host = hosts.find((candidate) => candidate.id === folder.value.deviceId)
  const recent = host?.places.find((group) => group.label === 'Recent')?.places ?? []
  return recent.map((place) => ({
    id: `${host!.id}:${place.path}`,
    path: place.path,
    disabled: !host!.online,
  }))
})
const workspaceName = computed(() => folder.value.path.split('/').filter(Boolean).at(-1) ?? null)
const mainHost = computed<HostMenuMainHost>(() => {
  const device = devices.find((candidate) => candidate.id === folder.value.deviceId)
  return device
    ? { id: device.id, name: device.name, kind: 'device' }
    : { id: 'cloud', name: 'Cloud', kind: 'cloud' }
})
const attachedHosts = ref<HostDeviceOption[]>([])
const locked = computed(() => session.phase !== 'idle' || session.archived)
async function selectFolder(deviceId: string, path: string): Promise<boolean> {
  folder.value = {
    deviceId,
    path,
  }
  return true
}
function selectRecent(id: string): void {
  const recent = recentDirectories.value.find((entry) => entry.id === id)
  if (recent) {
    folder.value = { deviceId: folder.value.deviceId, path: recent.path }
  }
}
function switchMain(id: string): void {
  const host = hosts.find((candidate) => candidate.id === id)
  if (host) {
    folder.value = { deviceId: host.id, path: host.source.home }
  }
}
function attach(id: string): void {
  const device = devices.find((candidate) => candidate.id === id)
  if (device && !attachedHosts.value.some((host) => host.id === id)) {
    attachedHosts.value = [...attachedHosts.value, device]
  }
}
function detach(id: string): void {
  attachedHosts.value = attachedHosts.value.filter((host) => host.id !== id)
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
            :workspace-name="workspaceName"
            :device-id="folder.deviceId"
            :locked="locked"
            browse-enabled
            :recent-directories="recentDirectories"
            :hosts="hosts"
            :select-folder="selectFolder"
            @select-recent="selectRecent"
          >
            <HostMenu
              :main-host="mainHost"
              :attached-hosts="attachedHosts"
              :devices="devices"
              :main-locked="locked"
              :attachments-locked="session.archived"
              @switch-main="switchMain"
              @attach="attach"
              @detach="detach"
            />
          </WorkspaceDirectoryMenu>
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

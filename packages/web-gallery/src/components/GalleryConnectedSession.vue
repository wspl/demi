<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import SidebarAccount from '@demicodes/web-ui/sidebar/SidebarAccount.vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import WorkspaceDirectoryMenu from '@demicodes/web-ui/hosts/WorkspaceDirectoryMenu.vue'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { transcriptDemoBlocks } from '../fixtures/blocks'
import { gallerySubagents } from '../fixtures/subagents'
import { galleryTerminals } from '../fixtures/terminals'
import { createGalleryFileHosts } from '../fixtures/files'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import GalleryComposer from './GalleryComposer.vue'
import { productWould } from '../product-would'
import { useLiveGalleryCommand } from '../live-command'
import { useTurnFlow } from '../turn-flow'

const props = withDefaults(defineProps<{ showActivity?: boolean }>(), { showActivity: true })
// The product asks the backend for a title and disables Detect Title until the
// user's next message; here a timer stands in for the model. As in the
// backend, a rename made while the model writes wins over its title.
const retitle = ref<'available' | 'running' | null>('available')
let retitleTimer: ReturnType<typeof setTimeout> | undefined
function updateTitle(): void {
  retitle.value = 'running'
  const asked = session.title
  clearTimeout(retitleTimer)
  retitleTimer = setTimeout(() => {
    if (session.title === asked) {
      session.title = asked === 'Login test fix' ? 'Session cookie rename' : 'Login test fix'
    }
    retitle.value = null
    // A message sent now would offer Detect Title again; the specimen does it after a pause.
    retitleTimer = setTimeout(() => { retitle.value = 'available' }, 3000)
  }, 1500)
}
onBeforeUnmount(() => clearTimeout(retitleTimer))

// The session runs on the gallery's scripted runtime: a message sent here gets a turn, as in the product.
const flow = useTurnFlow({
  id: 'shared-product-session',
  title: 'Shared session',
  cwd: WORKSPACE_ROOT,
  blocks: transcriptDemoBlocks(),
  subagents: props.showActivity ? gallerySubagents() : [],
  terminals: props.showActivity ? galleryTerminals() : [],
})
const session = flow.state
useLiveGalleryCommand(session.terminals)
// The session's messages find the files the gallery provides its transcripts.
const messageFiles = useMessageFiles()
const hosts = createGalleryFileHosts()
const devices: HostDeviceOption[] = hosts.map((host) => ({
  id: host.id,
  name: host.label,
  state: host.state,
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
    disabled: host!.state !== 'online',
  }))
})
const workspaceName = computed(() => folder.value.path.split('/').filter(Boolean).at(-1) ?? null)
const primaryHost = computed<HostMenuHost>(() => {
  const device = devices.find((candidate) => candidate.id === folder.value.deviceId)
  return device
    ? { id: device.id, name: device.name, kind: 'device', state: device.state }
    : { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online' }
})
const attachedHosts = ref<HostMenuHost[]>([])
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
/** A Host chosen in the host menu: a device moves the session to its home; the gallery has no Cloud to move to. */
function choose(chosen: { kind: 'cloud' } | { kind: 'device'; id: string }): void {
  if (chosen.kind === 'cloud') {
    productWould('Move the Conversation to the Cloud')
    return
  }
  const host = hosts.find((candidate) => candidate.id === chosen.id)
  if (host) {
    folder.value = { deviceId: host.id, path: host.source.home }
  }
}
function detach(id: string): void {
  attachedHosts.value = attachedHosts.value.filter((host) => host.id !== id)
}
</script>
<template>
  <div class="h-[36rem] overflow-hidden rounded-xl border border-border">
    <SidebarLayout label="Shared session">
      <template #sidebar>
        <div class="w-48 p-3">
          <SidebarAccount
            :account="{ name: '', email: 'new@example.com' }"
            @open-settings="productWould('Open Settings')"
            @sign-out="productWould('Sign Out')"
          />
        </div>
      </template>
      <ChatSession
        :conversation="session"
        :files="messageFiles()"
        @rename="session.title = $event"
        :retitle="retitle"
        @retitle="updateTitle"
        has-provider
        @save-scroll="(_id, state) => (session.scroll = state)"
        :pending-submission="flow.pendingSubmission.value"
        @retry-submission="flow.retrySubmission"
        @retry="flow.resume()"
        @abort-subagents="flow.abortSubagents"
        @abort-subagent="flow.abortSubagent"
        @abort-terminal="flow.abortTerminal"
        @remove-queued="flow.removeQueued"
        @send-queued="flow.sendQueued"
        @remove-pending-steer="flow.removePendingSteer"
        @interrupt-pending-steer="flow.interruptPendingSteer"
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
              :primary-host="primaryHost"
              :attached-hosts="attachedHosts"
              :devices="devices"
              :locked="locked"
              @choose="choose"
              @detach="detach"
              @connect="productWould('Add Device')"
            />
          </WorkspaceDirectoryMenu>
        </template>
        <template #composer
          ><GalleryComposer
            placeholder="Ask Demi…"
            :running="session.phase === 'running'"
            :compacting="session.phase === 'compacting'"
            :usage="session.contextUsage ?? undefined"
            :archived="session.archived"
            @restore="session.archived = false"
            :attachments="[{ name: 'ready-example.png', phase: 'ready' }]"
            @send="flow.turn"
            @steer="flow.steer"
            @queue="flow.queue"
            @stop="flow.stop"
            @compact="flow.compact"
        /></template>
      </ChatSession>
    </SidebarLayout>
  </div>
</template>

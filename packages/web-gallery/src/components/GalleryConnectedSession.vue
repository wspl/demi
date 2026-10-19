<script setup lang="ts">
import { computed, ref } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import SidebarAccount from '@demicodes/web-ui/sidebar/SidebarAccount.vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import WorkspaceDirectoryMenu from '@demicodes/web-ui/hosts/WorkspaceDirectoryMenu.vue'
import MoveConversationDialog from '@demicodes/web-ui/hosts/MoveConversationDialog.vue'
import { useMoveQuestion, workToStop } from '@demicodes/web-ui/hosts/move-question'
import { homePath } from '@demicodes/web-ui/files/paths'
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
import { useAgentContext } from '../agent-context'

/** What a child's context card in the Agents window shows and does beside its usage. */
const agentContext = useAgentContext()

const props = withDefaults(defineProps<{ showActivity?: boolean }>(), { showActivity: true })
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
// The hosts' marks are the ones the account's devices show, so the host menu and the folder
// browser draw each device alike.
const devices: HostDeviceOption[] = hosts.map((host) => ({
  id: host.id,
  name: host.label,
  state: host.state,
  mark: host.mark ?? null,
}))
const folder = ref({
  deviceId: hosts[0]!.id,
  path: hosts[0]!.source.home,
})
/**
 * The account's projects, by host and directory: each host's Recent places
 * to begin with, and every directory but a home the session moves to, which
 * the move makes a project as the product's does.
 */
const projectDirectories = ref(hosts.flatMap((host) =>
  (host.places.find((group) => group.label === 'Recent')?.places ?? []).map((place) => ({ deviceId: host.id, path: place.path })),
))
/** The projects on the session's host, which the directory menu lists. */
const hostProjects = computed(() => {
  const host = hosts.find((candidate) => candidate.id === folder.value.deviceId)
  return projectDirectories.value
    .filter((project) => project.deviceId === host?.id)
    .map((project) => ({
      id: `${project.deviceId}:${project.path}`,
      path: project.path,
      disabled: host!.state !== 'online',
    }))
})
/** The project the session runs in, which the menu checks; none in a home. */
const selectedProject = computed(() => hostProjects.value.find((entry) => entry.path === folder.value.path)?.id)

/** Puts the session in `path` on `deviceId`, which makes the directory a project unless it is the host's home. */
function settle(deviceId: string, path: string): void {
  folder.value = { deviceId, path }
  const home = hosts.find((candidate) => candidate.id === deviceId)?.source.home
  const known = projectDirectories.value.some((project) => project.deviceId === deviceId && project.path === path)
  if (path !== home && !known) {
    projectDirectories.value = [...projectDirectories.value, { deviceId, path }]
  }
}
const workspaceName = computed(() => folder.value.path.split('/').filter(Boolean).at(-1) ?? null)
const primaryHost = computed<HostMenuHost>(() => {
  const device = devices.find((candidate) => candidate.id === folder.value.deviceId)
  return device
    ? { id: device.id, name: device.name, kind: 'device', state: device.state, mark: device.mark }
    : { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null }
})
const attachedHosts = ref<HostMenuHost[]>([])
// A move stays open while the agent works, as the product's: choosing asks to stop it first.
const locked = computed(() => (session.archived ? 'archived' as const : undefined))
const moveQuestion = useMoveQuestion()

/**
 * Moves the session to `path` on `deviceId`: at once while the agent is
 * idle, otherwise once the user answers the question, which stops the agent,
 * its subagents and its commands first.
 */
async function moveTo(deviceId: string, path: string): Promise<boolean> {
  const host = hosts.find((candidate) => candidate.id === deviceId)
  // The specimen's Host is always online: its agent never only waits for it.
  const stops = workToStop({ ...session, onlyWaitsForHost: false })
  const move = async () => {
    flow.abortSubagents()
    for (const terminal of session.terminals) {
      flow.abortTerminal(terminal.id)
    }
    flow.stop()
    settle(deviceId, path)
    return true
  }
  if (!stops || !host) {
    settle(deviceId, path)
    return true
  }
  return moveQuestion.ask(
    {
      host: host.label,
      directory: homePath(path, host.source.home),
      from: primaryHost.value.name,
      fromCloud: primaryHost.value.kind === 'cloud',
      stops,
    },
    move,
  )
}
async function selectFolder(deviceId: string, path: string): Promise<boolean> {
  return moveTo(deviceId, path)
}
function selectProject(id: string): void {
  const project = hostProjects.value.find((entry) => entry.id === id)
  if (project) {
    void moveTo(folder.value.deviceId, project.path)
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
    void moveTo(host.id, host.source.home)
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
            :account="{ id: 'new', name: '', email: 'new@example.com' }"
            @open-settings="productWould('Open Settings')"
            @sign-out="productWould('Sign Out')"
          />
        </div>
      </template>
      <ChatSession @show-subagents="() => {}"
        :conversation="session"
        :files="messageFiles()"
        @rename="session.title = $event"
        has-provider
        @save-scroll="(_id, state) => (session.scroll = state)"
        :pending-submissions="flow.pendingSubmissions.value"
        :removing="flow.removing.value"
        @retry-submission="flow.retrySubmission"
        @edit-submission="flow.editSubmission"
        @delete-submission="flow.deleteSubmission"
        @retry="flow.resume()"
        @abort-subagents="flow.abortSubagents"
        @abort-subagent="flow.abortSubagent"
        @abort-terminal="flow.abortTerminal"
        @recall-queued="flow.recallQueued"
        @send-queued="flow.sendQueued"
        @recall-steer="flow.recallSteer"
        @interrupt-pending-steer="flow.interruptPendingSteer"
        v-bind="agentContext.session"
      >
        <template #workspace>
          <WorkspaceDirectoryMenu
            :overlay-store="appOverlayStore"
            :path="folder.path"
            :workspace-name="workspaceName"
            :device-id="folder.deviceId"
            :locked="locked !== undefined"
            browse-enabled
            :projects="hostProjects"
            :selected-project-id="selectedProject"
            :hosts="hosts"
            :select-folder="selectFolder"
            @select-project="selectProject"
          >
            <HostMenu
              :primary-host="primaryHost"
              conversation-id="gallery-connected-session"
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
            :running="session.phase === 'running' || flow.startingTurn.value"
            :compacting="session.phase === 'compacting'"
            :usage="session.contextUsage ?? undefined"
            :archived="session.archived"
            @restore="session.archived = false"
            :attachments="[{ name: 'ready-example.png', phase: 'ready' }]"
            :flow="flow"
            @send="flow.turn"
            @steer="flow.steer"
            @queue="flow.queue"
            @stop="flow.stop"
            @compact="flow.compact"
        /></template>
      </ChatSession>
    </SidebarLayout>
    <MoveConversationDialog
      :is-open="moveQuestion.open.value"
      :overlay-store="appOverlayStore"
      :question="moveQuestion.question.value"
      :answers="moveQuestion.answers"
      @close="moveQuestion.cancel"
      @move="moveQuestion.answer"
    />
  </div>
</template>

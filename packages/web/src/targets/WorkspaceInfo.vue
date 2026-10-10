<script setup lang="ts">
import { computed, ref } from 'vue'
import WorkspaceDirectoryMenu from '@demicodes/web-ui/hosts/WorkspaceDirectoryMenu.vue'
import MoveConversationDialog from '@demicodes/web-ui/hosts/MoveConversationDialog.vue'
import { useMoveQuestion } from '@demicodes/web-ui/hosts/move-question'
import { homePath } from '@demicodes/web-ui/files/paths'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import type { ConversationTarget } from '../api/generated/web-api'
import type { Conversation, Project } from '../state/types'
import { useResources } from '../state/resources'
import { browserHosts, fileSourceFor, placesFor } from '../devices/files'
import { useConversations } from '../conversation/store'
import { executionFor } from './execution'
import HostMenu from './HostMenu.vue'
import type { HostChoice } from '@demicodes/web-ui/hosts/types'

/**
 * The header's place: the Host the conversation runs on and its directory
 * (`product.md` § Where a conversation runs). Every move the two menus
 * start, to another Host or another directory, comes through `move`, which
 * asks first when the conversation has messages; so does one the offline
 * primary Host's card above the composer starts, through `choose`.
 */
const props = defineProps<{
  project?: Project
  conversation: Conversation
}>()
const resources = useResources()
const conversations = useConversations()
const directory = ref<InstanceType<typeof WorkspaceDirectoryMenu>>()
const execution = computed(() => executionFor(props.conversation))
const locked = computed(
  () =>
    props.conversation.phase !== 'idle' ||
    props.conversation.archived ||
    conversations.pendingChanges.includes(props.conversation.id),
)
/**
 * A turn that waits for the offline primary Host may move all the same
 * (`sessions-and-targets.md` § Switch the primary target); whether the
 * turn does nothing but wait is the backend's to check, which refuses the
 * move otherwise. Its agent is told of the move whatever the user chose.
 */
const waiting = computed(
  () =>
    props.conversation.phase !== 'idle' &&
    execution.value.kind === 'device' &&
    execution.value.state === 'offline',
)
const moveLocked = computed(() => locked.value && !waiting.value)
const recentDirectories = computed(() =>
  resources.recentProjectIds
    .flatMap(
      (id) => resources.projects.find((project) => project.id === id) ?? [],
    )
    .filter((project) => project.deviceId === execution.value.deviceId)
    .slice(0, 8)
    .map((project) => ({
      id: project.id,
      path: project.path,
      disabled: project.hostKind === 'device' && project.state !== 'online',
    })),
)
const hosts = computed(() =>
  browserHosts(resources.devices).map((host) => {
    const device = resources.devices.find((device) => device.id === host.id)!
    return {
      ...host,
      source: fileSourceFor(device),
      places: placesFor(device, resources.projects),
    }
  }),
)

const moveQuestion = useMoveQuestion()

/** Where `target` is, as the dialog names it: its Host, and its directory as a person reads it. */
function place(target: ConversationTarget): { host: string; directory: string | null } {
  if (target.kind === 'cloud') {
    // Without a directory the Cloud runs it in one of its own, which the backend names.
    return { host: 'Cloud', directory: target.path ?? null }
  }
  const project = target.kind === 'workspace'
    ? resources.projects.find((item) => item.id === target.workspaceId)
    : undefined
  const deviceId = target.kind === 'device' ? target.deviceId : project?.deviceId ?? null
  const path = target.kind === 'device' ? target.path : project?.path ?? null
  const device = resources.deviceById(deviceId)
  const host = device?.kind === 'managed' ? 'Cloud' : (device?.name ?? 'Unavailable device')
  return { host, directory: path === null ? null : homePath(path, device?.home) }
}

function sameTarget(a: ConversationTarget, b: ConversationTarget): boolean {
  switch (a.kind) {
    case 'cloud':
      return b.kind === 'cloud' && a.path === b.path
    case 'device':
      return b.kind === 'device' && a.deviceId === b.deviceId && a.path === b.path
    case 'workspace':
      return b.kind === 'workspace' && a.workspaceId === b.workspaceId
  }
}

/**
 * Moves the conversation to `target`: at once while it has sent nothing,
 * otherwise once the user answers the dialog. Answers whether it moved.
 */
function move(target: ConversationTarget): Promise<boolean> {
  if (moveLocked.value) {
    return Promise.resolve(false)
  }
  // Where it runs already is no move.
  if (sameTarget(target, props.conversation.target)) {
    return Promise.resolve(true)
  }
  if (props.conversation.persistence !== 'synced') {
    return conversations.switchTarget(props.conversation.id, target)
  }
  // The waiting turn is told either way, so there is nothing to ask.
  if (waiting.value) {
    return conversations.switchTarget(props.conversation.id, target, true)
  }
  return moveQuestion.ask(
    { ...place(target), from: execution.value.name, fromCloud: execution.value.kind === 'cloud' },
    (tell) => conversations.switchTarget(props.conversation.id, target, tell),
  )
}

/**
 * A Host chosen in the host menu. Outside a project the conversation moves
 * there at once, to a device's home or the Cloud's directory of the
 * conversation; in a project a device opens its directory picker. The Cloud
 * lists no directories to a page, so choosing it moves the conversation to
 * that directory of its own in either case.
 */
function choose(host: HostChoice) {
  const here = execution.value
  if (host.kind === 'cloud') {
    if (here.kind !== 'cloud') {
      void move({ kind: 'cloud' })
    }
    return
  }
  if (here.kind === 'device' && here.deviceId === host.id) {
    return
  }
  const home = resources.deviceById(host.id)?.home
  if (props.conversation.target.kind === 'workspace' || !home) {
    directory.value?.browse(host.id)
    return
  }
  void move({ kind: 'device', deviceId: host.id, path: home })
}

async function selectRecent(id: string) {
  if (await move({ kind: 'workspace', workspaceId: id })) {
    resources.rememberProject(id)
  }
}

/** A directory chosen in the picker: the project whose directory it is, or the directory itself. */
async function selectFolder(deviceId: string, path: string): Promise<boolean> {
  const project = resources.projects.find(
    (project) => project.deviceId === deviceId && project.path === path,
  )
  const moved = await move(
    project
      ? { kind: 'workspace', workspaceId: project.id }
      : { kind: 'device', deviceId, path },
  )
  if (moved && project) {
    resources.rememberProject(project.id)
  }
  return moved
}

defineExpose({ choose })
</script>
<template>
  <WorkspaceDirectoryMenu
    ref="directory"
    :overlay-store="appOverlayStore"
    :path="execution.directory"
    :workspace-name="execution.workspaceName"
    :selected-project-id="project?.id"
    :device-id="execution.deviceId"
    :locked="moveLocked"
    :browse-enabled="execution.kind !== 'cloud'"
    :recent-directories="recentDirectories"
    :hosts="hosts"
    :select-folder="selectFolder"
    @select-recent="selectRecent"
  >
    <HostMenu
      :conversation="conversation"
      :locked="locked"
      @choose="choose"
    />
  </WorkspaceDirectoryMenu>
  <MoveConversationDialog
    :is-open="moveQuestion.open.value"
    :overlay-store="appOverlayStore"
    :question="moveQuestion.question.value"
    :busy="moveQuestion.busy.value"
    @close="moveQuestion.cancel"
    @move="moveQuestion.answer"
  />
</template>

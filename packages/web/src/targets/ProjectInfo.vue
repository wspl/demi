<script setup lang="ts">
import { keptComputed } from '@demicodes/web-ui/infra/keep-same'
import { computed, ref } from 'vue'
import ProjectDirectoryMenu from '@demicodes/web-ui/targets/ProjectDirectoryMenu.vue'
import MoveConversationDialog from '@demicodes/web-ui/targets/MoveConversationDialog.vue'
import { useMoveQuestion } from '@demicodes/web-ui/targets/move-question'
import { homePath } from '@demicodes/web-ui/files/paths'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import type { ConversationTarget } from '../api/generated/web-api'
import type { Conversation, Project } from '../state/types'
import { useResources } from '../state/resources'
import { targetKindOf, deviceLabel } from '../state/devices'
import { browserDevices, fileSourceFor, placesFor } from '../devices/files'
import { useConversations } from '../conversation/store'
import { executionFor, moveLock } from './execution'
import DeviceMenu from './DeviceMenu.vue'
import type { DeviceChoice } from '@demicodes/web-ui/targets/types'

/**
 * The header's place: the device the conversation runs on and its directory.
 * Every move the two menus start, to another device or another directory, comes
 * through `move`, which asks first when the conversation has messages; so does
 * one the offline primary device's card above the composer starts, through
 * `choose`. A move stops nothing.
 */
const props = defineProps<{
  project?: Project
  conversation: Conversation
}>()
const resources = useResources()
const conversations = useConversations()
const directory = ref<InstanceType<typeof ProjectDirectoryMenu>>()
const execution = keptComputed(() => executionFor(props.conversation))
const locked = computed(() => moveLock(props.conversation, conversations.pendingChanges))
/** The projects of the conversation's workspace, which its moves choose among. */
const workspaceProjects = computed(() =>
  resources.projects.filter((project) => project.workspaceId === props.conversation.workspaceId),
)
/** The projects on the device the conversation runs on, which the directory menu lists. */
const deviceProjects = keptComputed(() =>
  workspaceProjects.value
    .filter((project) => project.deviceId === execution.value.deviceId)
    .map((project) => ({
      id: project.id,
      path: project.path,
      disabled: project.targetKind === 'device' && !(project.device.kind === 'paired' && project.device.state === 'online'),
    })),
)
const devices = keptComputed(() =>
  browserDevices(resources.devices).map((offered) => {
    const device = resources.devices.find((each) => each.id === offered.id)!
    return {
      ...offered,
      source: fileSourceFor(device),
      places: placesFor(device, workspaceProjects.value),
    }
  }),
)

const moveQuestion = useMoveQuestion()

/** Where `target` is, as the dialog names it: its device, and its directory as a person reads it. */
function place(target: ConversationTarget): { device: string; directory: string | null } {
  if (target.kind === 'cloud') {
    // Without a directory the Cloud runs it in one of its own, which the backend names.
    return { device: deviceLabel('cloud', null), directory: target.path ?? null }
  }
  const project = target.kind === 'project'
    ? resources.projects.find((item) => item.id === target.projectId)
    : undefined
  const deviceId = target.kind === 'device' ? target.deviceId : project?.deviceId ?? null
  const path = target.kind === 'device' ? target.path : project?.path ?? null
  const device = resources.deviceById(deviceId)
  const label = deviceLabel(targetKindOf(device), device)
  return { device: label, directory: path === null ? null : homePath(path, device?.home) }
}

function sameTarget(a: ConversationTarget, b: ConversationTarget): boolean {
  switch (a.kind) {
    case 'cloud':
      return b.kind === 'cloud' && a.path === b.path
    case 'device':
      return b.kind === 'device' && a.deviceId === b.deviceId && a.path === b.path
    case 'project':
      return b.kind === 'project' && a.projectId === b.projectId
  }
}

/**
 * Moves the conversation to `target`: at once while it has sent nothing,
 * otherwise once the user answers the dialog. Answers whether it moved.
 */
function move(target: ConversationTarget): Promise<boolean> {
  if (locked.value !== undefined) {
    return Promise.resolve(false)
  }
  // Where it runs already is no move; its project, while it works in a
  // worktree of it, is the move back to the project's directory.
  if (sameTarget(target, props.conversation.target) && !(target.kind === 'project' && props.conversation.worktree)) {
    return Promise.resolve(true)
  }
  if (props.conversation.persistence !== 'synced') {
    return conversations.switchTarget(props.conversation.id, target)
  }
  return moveQuestion.ask(
    {
      ...place(target),
      from: execution.value.name,
      fromCloud: execution.value.kind === 'cloud',
    },
    () => conversations.switchTarget(props.conversation.id, target),
  )
}

/**
 * A device chosen in the device menu. Outside a project the conversation moves
 * there at once, to a device's home or the Cloud's directory of the
 * conversation; in a project a device opens its directory picker. The Cloud
 * lists no directories to a page, so choosing it moves the conversation to
 * that directory of its own in either case.
 */
function choose(choice: DeviceChoice) {
  const here = execution.value
  if (choice.kind === 'cloud') {
    if (here.kind !== 'cloud') {
      void move({ kind: 'cloud' })
    }
    return
  }
  if (here.kind === 'device' && here.deviceId === choice.id) {
    return
  }
  const home = resources.deviceById(choice.id)?.home
  if (props.conversation.target.kind === 'project' || !home) {
    directory.value?.browse(choice.id)
    return
  }
  void move({ kind: 'device', deviceId: choice.id, path: home })
}

function selectProject(id: string) {
  void move({ kind: 'project', projectId: id })
}

/** A directory chosen in the picker: the project whose directory it is, or the directory itself. */
function selectFolder(deviceId: string, path: string): Promise<boolean> {
  const project = workspaceProjects.value.find(
    (project) => project.deviceId === deviceId && project.path === path,
  )
  return move(
    project
      ? { kind: 'project', projectId: project.id }
      : { kind: 'device', deviceId, path },
  )
}

defineExpose({ choose })
</script>
<template>
  <ProjectDirectoryMenu
    ref="directory"
    :overlay-store="appOverlayStore"
    :path="execution.directory"
    :project-name="execution.projectName"
    :selected-project-id="project?.id"
    :device-id="execution.deviceId"
    :locked="locked !== undefined"
    :browse-enabled="execution.kind !== 'cloud'"
    :projects="deviceProjects"
    :devices="devices"
    :worktree="conversation.worktree ? { name: conversation.worktree, path: conversation.cwd } : null"
    :select-folder="selectFolder"
    @select-project="selectProject"
  >
    <DeviceMenu
      :conversation="conversation"
      :locked="locked"
      @choose="choose"
    />
  </ProjectDirectoryMenu>
  <MoveConversationDialog
    :is-open="moveQuestion.open.value"
    :overlay-store="appOverlayStore"
    :question="moveQuestion.question.value"
    :answers="moveQuestion.answers"
    @close="moveQuestion.cancel"
    @move="moveQuestion.answer"
  />
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import WorkspaceDirectoryMenu from '@demicodes/web-ui/hosts/WorkspaceDirectoryMenu.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import type { Conversation, Project } from '../state/types'
import { useResources } from '../state/resources'
import { browserHosts, fileSourceFor, placesFor } from '../devices/files'
import { useConversations } from '../conversation/store'
import { executionFor } from './execution'
import HostMenu from './HostMenu.vue'
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
      disabled:
        project.hostKind !== 'cloud' &&
        !resources.devices.find((device) => device.id === project.deviceId)
          ?.online,
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
async function browse(deviceId: string, cwd?: string | null) {
  if (locked.value) {
    return
  }
  if (deviceId === 'cloud') {
    await conversations.switchTarget(props.conversation.id, { kind: 'cloud' })
    return
  }
  directory.value?.browse(deviceId, cwd)
}
async function selectRecent(id: string) {
  if (locked.value) {
    return
  }
  if (await conversations.move([props.conversation.id], id)) {
    resources.rememberProject(id)
  }
}
async function selectFolder(deviceId: string, path: string): Promise<boolean> {
  if (locked.value) {
    return false
  }
  const project = resources.projects.find(
    (project) => project.deviceId === deviceId && project.path === path,
  )
  const switched = project
    ? await conversations.move([props.conversation.id], project.id)
    : await conversations.switchTarget(props.conversation.id, {
        kind: 'device',
        deviceId,
        path,
      })
  if (switched && project) {
    resources.rememberProject(project.id)
  }
  return switched
}
</script>
<template>
  <WorkspaceDirectoryMenu
    ref="directory"
    :overlay-store="appOverlayStore"
    :path="execution.path"
    :workspace-name="execution.workspaceName"
    :selected-project-id="project?.id"
    :device-id="execution.deviceId"
    :locked="locked"
    :browse-enabled="execution.kind !== 'cloud'"
    :recent-directories="recentDirectories"
    :hosts="hosts"
    :select-folder="selectFolder"
    @select-recent="selectRecent"
  >
    <HostMenu
      :conversation="conversation"
      :project="project"
      @switch-main="browse"
    />
  </WorkspaceDirectoryMenu>
</template>

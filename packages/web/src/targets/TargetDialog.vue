<script setup lang="ts">
import { computed, ref } from 'vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDraft } from '@demicodes/web-ui/hosts/workspace'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useConversations } from '../conversation/store'
import { useResources } from '../prototype/resources'
import { baseName } from '@demicodes/web-ui/files/paths'
import { CLOUD_HOME, fileSourceFor, placesFor } from '../prototype/files'

/** The shared working-environment dialog over the prototype's projects and devices. */
const props = defineProps<{ conversationId: string | null }>()
const resources = useResources()
const conversations = useConversations()
const current = computed(() => conversations.items.find((c) => c.id === props.conversationId))
const message = ref('')

const projects = computed(() => resources.projects.map((project) => ({ id: project.id, name: project.name, host: project.host, path: project.path })))
const devices = computed(() => resources.devices.map((device) => ({ id: device.id, name: device.name, online: device.online })))
const deviceById = (id: string) => resources.devices.find((device) => device.id === id) ?? null

function close() {
  resources.targetOpen = false
  message.value = ''
}

function select(id: string | null) {
  if (!current.value) return
  conversations.move([current.value.id], id)
  close()
}

/** A Cloud project is a managed workspace under the Cloud home, named as typed; a device project is its directory. */
function create(draft: WorkspaceDraft) {
  const device = draft.kind === 'device' ? deviceById(draft.deviceId) : null
  if (draft.kind === 'device' && !device) {
    message.value = 'Select a device and enter an absolute directory path.'
    return
  }
  const id = crypto.randomUUID()
  resources.projects.push({
    id,
    name: draft.kind === 'cloud' ? draft.name : baseName(draft.path) || 'Workspace',
    deviceId: draft.kind === 'cloud' ? 'cloud' : draft.deviceId,
    host: draft.kind === 'cloud' ? 'Cloud' : device!.name,
    hostKind: draft.kind,
    path: draft.kind === 'cloud' ? `${CLOUD_HOME}/${draft.name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-')}` : draft.path,
    branch: null,
  })
  if (resources.targetMode === 'switch') select(id)
  else close()
}
</script>

<template>
  <WorkspaceDialog
    :is-open="resources.targetOpen"
    :overlay-store="appOverlayStore"
    :mode="resources.targetMode"
    :projects="projects"
    :current-project-id="current?.projectId ?? null"
    :locked="!!current?.stream"
    :devices="devices"
    cloud
    :message="message"
    :source-for="(id) => fileSourceFor(deviceById(id))"
    :places-for="(id) => placesFor(deviceById(id), resources.projects)"
    @close="close"
    @select="select"
    @create="create"
  />
</template>

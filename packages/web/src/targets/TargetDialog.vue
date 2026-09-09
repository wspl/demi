<script setup lang="ts">
import { computed, ref } from 'vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDraft } from '@demicodes/web-ui/hosts/workspace'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useConversations } from '../conversation/store'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { baseName } from '@demicodes/web-ui/files/paths'
import { fileSourceFor, placesFor } from '../devices/files'

const props = defineProps<{ conversationId: string | null }>()
const resources = useResources()
const product = useProduct()
const conversations = useConversations()
const current = computed(() =>
  conversations.items.find(
    (conversation) => conversation.id === props.conversationId,
  ),
)
const message = ref('')
const pending = ref(false)
const projects = computed(() =>
  resources.projects.map((project) => ({
    id: project.id,
    name: project.name,
    host: project.host,
    path: project.path,
  })),
)
const deviceById = (id: string) =>
  resources.devices.find((device) => device.id === id) ?? null

function close() {
  resources.targetOpen = false
  message.value = ''
}

async function select(id: string | null) {
  if (!current.value || pending.value) {
    return
  }
  pending.value = true
  try {
    if (await conversations.move([current.value.id], id)) {
      close()
    }
  } finally {
    pending.value = false
  }
}

async function create(draft: WorkspaceDraft) {
  if (pending.value) {
    return
  }
  const conversationId = current.value?.id
  const mode = resources.targetMode
  pending.value = true
  message.value = ''
  try {
    const id = await resources.createProject(
      draft.kind === 'cloud'
        ? {
            cloud: true,
            name: draft.name,
          }
        : {
            deviceId: draft.deviceId,
            path: draft.path,
            name: baseName(draft.path) || 'Workspace',
          },
    )
    if (mode === 'switch' && conversationId) {
      if (!(await conversations.move([conversationId], id))) {
        message.value =
          'Project created. The conversation could not switch; select the project to try again.'
        return
      }
    }
    close()
  } catch (error) {
    message.value = error instanceof Error ? error.message : String(error)
  } finally {
    pending.value = false
  }
}
</script>

<template>
  <WorkspaceDialog
    :is-open="resources.targetOpen"
    :overlay-store="appOverlayStore"
    :mode="resources.targetMode"
    :projects="projects"
    :current-project-id="current?.projectId ?? null"
    :locked="pending || (current?.phase !== 'idle' && !!current)"
    :pending="pending"
    :devices="resources.devices"
    :cloud="!!product.snapshot?.cloud"
    :message="message"
    :source-for="(id) => fileSourceFor(deviceById(id))"
    :places-for="(id) => placesFor(deviceById(id), resources.projects)"
    @close="close"
    @select="select"
    @create="create"
    @connect-device="resources.pairingOpen = true"
  />
</template>

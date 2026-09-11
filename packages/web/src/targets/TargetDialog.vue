<script setup lang="ts">
import { ref } from 'vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDraft } from '@demicodes/web-ui/hosts/workspace'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { baseName } from '@demicodes/web-ui/files/paths'
import { fileSourceFor, placesFor } from '../devices/files'

const resources = useResources()
const product = useProduct()
const message = ref('')
const pending = ref(false)
const deviceById = (id: string) =>
  resources.devices.find((device) => device.id === id) ?? null

function close() {
  resources.targetOpen = false
  message.value = ''
}

async function create(draft: WorkspaceDraft) {
  if (pending.value) {
    return
  }
  pending.value = true
  message.value = ''
  try {
    await resources.createProject(
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
    :pending="pending"
    :load="product.load"
    @retry="product.revalidate"
    :devices="resources.devices"
    :cloud="!!product.snapshot?.cloud"
    :message="message"
    :source-for="(id) => fileSourceFor(deviceById(id))"
    :places-for="(id) => placesFor(deviceById(id), resources.projects)"
    @close="close"
    @create="create"
    @connect-device="resources.pairingOpen = true"
  />
</template>

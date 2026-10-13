<script setup lang="ts">
import { shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { ref } from 'vue'
import WorkspaceDialog from '@demicodes/web-ui/hosts/WorkspaceDialog.vue'
import type { WorkspaceDraft } from '@demicodes/web-ui/hosts/workspace'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useResources } from '../state/resources'
import { useConversationNavigation } from '../conversation/navigation'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { baseName } from '@demicodes/utils'
import { fileSourceFor, placesFor } from '../devices/files'

const resources = useResources()
const product = useProduct()
const preferences = usePreferences()
const navigation = useConversationNavigation()
const message = ref<ShownError | null>(null)
const deviceById = (id: string) =>
  resources.devices.find((device) => device.id === id) ?? null

function close() {
  resources.targetOpen = false
  message.value = null
}

/** Create: the dialog's button works until the project is made; done, the dialog closes if it is still open. */
async function create(draft: WorkspaceDraft): Promise<void> {
  message.value = null
  try {
    await navigation.createProject(
      draft.kind === 'cloud'
        ? {
            kind: 'cloud',
            name: draft.name,
          }
        : {
            kind: 'device',
            deviceId: draft.deviceId,
            path: draft.path,
            name: baseName(draft.path) || 'Workspace',
          },
    )
    if (resources.targetOpen) {
      close()
    }
  } catch (error) {
    message.value = shownError(error)
  }
}
</script>

<template>
  <WorkspaceDialog
    :is-open="resources.targetOpen"
    :overlay-store="appOverlayStore"
    :load="product.load"
    @retry="product.reconnect"
    :devices="resources.devices"
    :last-host="preferences.lastProjectHost"
    :message="message"
    :source-for="(id) => fileSourceFor(deviceById(id))"
    :places-for="(id) => placesFor(deviceById(id), resources.projects)"
    @close="close"
    @create="create"
    @choose="(choice) => preferences.update({ lastProjectHost: choice }, true)"
    @connect-device="resources.openPairing"
  />
</template>

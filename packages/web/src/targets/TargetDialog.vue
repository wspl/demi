<script setup lang="ts">
import { shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { ref } from 'vue'
import ProjectDialog from '@demicodes/web-ui/hosts/ProjectDialog.vue'
import type { ProjectDraft } from '@demicodes/web-ui/hosts/project'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useResources } from '../state/resources'
import { useConversationNavigation } from '../conversation/navigation'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { t as shared } from '@demicodes/web-ui/locales/text'
import { fileSourceFor, placesFor } from '../devices/files'
import { useWorkspaces } from '../state/workspaces'

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
const workspaces = useWorkspaces()
async function create(draft: ProjectDraft): Promise<void> {
  message.value = null
  try {
    await navigation.createProject(
      draft.kind === 'cloud'
        ? {
            kind: 'cloud',
            workspaceId: workspaces.current ?? '',
            name: draft.name,
          }
        : {
            kind: 'device',
            workspaceId: workspaces.current ?? '',
            deviceId: draft.deviceId,
            path: draft.path,
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
  <ProjectDialog
    :is-open="resources.targetOpen"
    :overlay-store="appOverlayStore"
    :load="product.partLoad('devices')"
    @retry="product.reconnect"
    :devices="resources.devices"
    :last-host="preferences.lastProjectHost"
    :message="message"
    :source-for="(id) => fileSourceFor(deviceById(id))"
    :places-for="(id) => placesFor(deviceById(id), resources.projects)"
    @close="close"
    @create="create"
    @choose="(choice) => preferences.update({ lastProjectHost: choice })"
    @connect-device="resources.openPairing"
  />
</template>

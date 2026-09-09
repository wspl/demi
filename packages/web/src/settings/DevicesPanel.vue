<script setup lang="ts">
import { cloud, resetCloud } from '../prototype/cloud'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'
import { claimDevice, deviceInstallation } from '../prototype/pairing'

const resources = useResources()
const conversations = useConversations()

function revoke(id: string) {
  if (resources.projects.some((p) => p.deviceId === id)) {
    showToast(
      {
        title: 'Device is in use',
        message: 'Remove the projects using this device before revoking it.',
        tone: 'danger'
      }
    )
    return
  }
  resources.devices = resources.devices.filter((d) => d.id !== id)
  for (const c of conversations.items)
    c.attachedHosts = c.attachedHosts.filter((host) => host.deviceId !== id)
}
</script>

<template>
  <SettingsDevices
    :devices="resources.devices"
    :cloud="cloud"
    @reset-cloud="resetCloud"
    :overlay-store="appOverlayStore"
    :installation="deviceInstallation"
    :claim-device="claimDevice"
    @revoke="revoke"
  />
</template>

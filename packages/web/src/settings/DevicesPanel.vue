<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import type { PairingResult } from '@demicodes/web-ui/devices/pairing'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'

const resources = useResources()
const conversations = useConversations()
// Prototype endpoint contract; installers are not served by this SPA.
const installation = {
  shellInstallerUrl: `${window.location.origin}/install.sh`,
  powershellInstallerUrl: `${window.location.origin}/install.ps1`,
}

async function claim(_code: string): Promise<PairingResult> {
  const device = {
    id: crypto.randomUUID(),
    name: `host-${resources.devices.length + 1}`,
    online: true,
    home: '/home/demo',
  }
  resources.devices.push(device)
  return { ok: true, device }
}

function revoke(id: string) {
  if (resources.projects.some((p) => p.deviceId === id)) {
    showToast({ title: 'Device is in use', message: 'Remove the projects using this device before revoking it.', tone: 'danger' })
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
    :overlay-store="appOverlayStore"
    :installation="installation"
    :claim-device="claim"
    @revoke="revoke"
  />
</template>

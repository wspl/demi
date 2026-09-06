<script setup lang="ts">
import { ref } from 'vue'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'

const resources = useResources()
const conversations = useConversations()
const message = ref('')

function claim(name: string) {
  resources.devices.push({ id: crypto.randomUUID(), name, online: true, home: '/home/demo' })
  message.value = 'Device connected.'
}

function toggleOnline(id: string) {
  const device = resources.devices.find((d) => d.id === id)
  if (device) device.online = !device.online
}

function revoke(id: string) {
  if (resources.projects.some((p) => p.deviceId === id)) {
    message.value = 'Remove the projects using this device before revoking it.'
    return
  }
  resources.devices = resources.devices.filter((d) => d.id !== id)
  for (const c of conversations.items)
    c.attachedHosts = c.attachedHosts.filter((host) => host.deviceId !== id)
  message.value = 'Device revoked.'
}
</script>

<template>
  <SettingsDevices
    :devices="resources.devices"
    :message="message"
    @toggle-online="toggleOnline"
    @revoke="revoke"
    @add="claim"
  />
</template>

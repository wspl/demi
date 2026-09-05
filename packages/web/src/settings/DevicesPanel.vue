<script setup lang="ts">
import Button from '@demicodes/web-ui/ui/Button.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'

import { ref } from 'vue'
import { Monitor, Plus } from '@lucide/vue'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'

const resources = useResources()
const conversations = useConversations()
const name = ref('')
const message = ref('')

function claim() {
  if (!name.value.trim()) return
  resources.devices.push({ id: crypto.randomUUID(), name: name.value.trim(), online: true })
  name.value = ''
  message.value = 'Demo device connected.'
}

function revoke(id: string) {
  if (resources.projects.some((p) => p.deviceId === id)) {
    message.value = 'Remove the projects using this device before revoking it.'
    return
  }
  resources.devices = resources.devices.filter((d) => d.id !== id)
  for (const c of conversations.items)
    c.attachedHosts = c.attachedHosts.filter((host) => host !== id)
  message.value = 'Device revoked.'
}
</script>

<template>
  <h3>Your devices</h3>
  <p class="hint">Connect a machine to work with its files. Connections here are simulated.</p>
  <div v-for="device in resources.devices" :key="device.id" class="resource-row">
    <Monitor :size="17" />
    <div class="resource-description">
      <strong>{{ device.name }}</strong>
      <span>{{ device.online ? 'Online' : 'Offline' }}</span>
    </div>
    <Button @click="device.online = !device.online">
      {{ device.online ? 'Go offline' : 'Connect' }}
    </Button>
    <Button @click="revoke(device.id)">Revoke</Button>
  </div>
  <p v-if="!resources.devices.length" class="empty-note">No devices connected.</p>
  <form class="add-resource" @submit.prevent="claim">
    <label>
      Demo device name
      <TextInput v-model="name" placeholder="My laptop" required maxlength="64" />
    </label>
    <Button @click="claim" :disabled="!name.trim()">
      <Plus :size="14" />
      Add demo device
    </Button>
  </form>
  <p v-if="message" role="status" class="hint">{{ message }}</p>
</template>

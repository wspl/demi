<script setup lang="ts">
import { ref } from 'vue'
import { Monitor, Plus } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SettingsDevice } from './types'

defineProps<{
  devices: SettingsDevice[]
  /** Outcome of the last action, shown under the form. */
  message?: string
}>()

const emit = defineEmits<{
  toggleOnline: [id: string]
  revoke: [id: string]
  add: [name: string]
}>()

const name = ref('')

function add() {
  const value = name.value.trim()
  if (!value) return
  emit('add', value)
  name.value = ''
}
</script>

<template>
  <h3>Your devices</h3>
  <div v-for="device in devices" :key="device.id" class="resource-row">
    <Monitor :size="ICON_PX.in28" class="text-fg-muted" />
    <div class="resource-description">
      <strong>{{ device.name }}</strong>
      <span>{{ device.online ? 'Online' : 'Offline' }}</span>
    </div>
    <div class="resource-actions">
      <Button @click="emit('toggleOnline', device.id)">{{ device.online ? 'Go offline' : 'Connect' }}</Button>
      <Button @click="emit('revoke', device.id)">Revoke</Button>
    </div>
  </div>
  <p v-if="!devices.length" class="empty-note">No devices connected.</p>
  <form class="add-resource" @submit.prevent="add">
    <label>
      Device name
      <TextInput v-model="name" placeholder="My laptop" required maxlength="64" />
    </label>
    <Button :disabled="!name.trim()" @click="add">
      <Plus :size="ICON_PX.in28" />
      Add device
    </Button>
  </form>
  <p v-if="message" role="status" class="hint">{{ message }}</p>
</template>

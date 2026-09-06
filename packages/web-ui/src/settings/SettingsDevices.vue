<script setup lang="ts">
import { ref } from 'vue'
import { Monitor } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsGroup from './SettingsGroup.vue'
import SettingsNote from './SettingsNote.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsDevice } from './types'

defineProps<{
  devices: SettingsDevice[]
  /** Outcome of the last action. */
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
  <SettingsPage title="Devices" description="Machines that can host a conversation's working directory.">
    <SettingsGroup title="Your devices">
      <SettingsRow
        v-for="device in devices"
        :key="device.id"
        :label="device.name"
        :description="device.online ? 'Online' : 'Offline'"
      >
        <template #leading>
          <span class="relative flex">
            <Monitor :size="ICON_PX.in28" />
            <span
              class="absolute -right-0.5 -top-0.5 size-1.5 rounded-full ring-2 ring-surface-float"
              :class="device.online ? 'bg-on-success' : 'bg-fg-ghost'"
            />
          </span>
        </template>
        <Button variant="ghost" size="sm" @click="emit('toggleOnline', device.id)">
          {{ device.online ? 'Go offline' : 'Connect' }}
        </Button>
        <Button variant="ghost" size="sm" @click="emit('revoke', device.id)">Revoke</Button>
      </SettingsRow>
      <div v-if="!devices.length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">
        No devices connected.
      </div>
    </SettingsGroup>
    <SettingsGroup title="Add a device" description="Claim a machine by name; it appears above once it connects.">
      <form @submit.prevent="add">
        <SettingsRow label="Device name">
          <TextInput v-model="name" placeholder="My laptop" maxlength="64" class="w-56 max-w-full" />
          <Button :disabled="!name.trim()" @click="add">Add device</Button>
        </SettingsRow>
      </form>
    </SettingsGroup>
    <SettingsNote v-if="message" :text="message" />
  </SettingsPage>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import type { SettingsProvider } from './types'

defineProps<{
  providers: SettingsProvider[]
  message?: string
}>()

const emit = defineEmits<{
  test: [id: string]
  remove: [id: string]
  setAvailable: [id: string, available: boolean]
  add: [label: string]
}>()

const label = ref('')

function add() {
  const value = label.value.trim()
  if (!value) return
  emit('add', value)
  label.value = ''
}
</script>

<template>
  <h3>Providers</h3>
  <div v-for="provider in providers" :key="provider.id">
    <div class="resource-row">
      <div class="resource-description">
        <strong>{{ provider.label }}</strong>
        <span>{{ provider.modelCount }} models · {{ provider.isAvailable ? 'Available' : 'Unavailable' }}</span>
      </div>
      <div class="resource-actions">
        <Button @click="emit('test', provider.id)">Test</Button>
        <Button @click="emit('remove', provider.id)">Remove</Button>
      </div>
    </div>
    <Checkbox
      :model-value="provider.isAvailable"
      label="Available for conversations"
      @update:model-value="(value) => emit('setAvailable', provider.id, value)"
    />
  </div>
  <p v-if="!providers.length" class="empty-note">No providers.</p>
  <form class="add-resource" @submit.prevent="add">
    <label>
      New provider
      <TextInput v-model="label" placeholder="Provider name" required maxlength="64" />
    </label>
    <Button :disabled="!label.trim()" @click="add">Add provider</Button>
  </form>
  <p v-if="message" role="status" class="hint">{{ message }}</p>
</template>

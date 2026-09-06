<script setup lang="ts">
import { ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsNote from './SettingsNote.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
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
  <SettingsPage title="Providers" description="Where conversations get their models. Off keeps a provider configured but out of the picker.">
    <SettingsGroup title="Providers">
      <SettingsRow
        v-for="provider in providers"
        :key="provider.id"
        :label="provider.label"
        :description="`${provider.modelCount} ${provider.modelCount === 1 ? 'model' : 'models'} · ${provider.isAvailable ? 'Available' : 'Unavailable'}`"
      >
        <Button variant="ghost" size="sm" @click="emit('test', provider.id)">Test</Button>
        <Button variant="ghost" size="sm" @click="emit('remove', provider.id)">Remove</Button>
        <Switch
          :model-value="provider.isAvailable"
          size="sm"
          class="ml-1"
          @update:model-value="(value) => emit('setAvailable', provider.id, value)"
        />
      </SettingsRow>
      <div v-if="!providers.length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">
        No providers yet.
      </div>
    </SettingsGroup>
    <SettingsGroup title="Add a provider">
      <form @submit.prevent="add">
        <SettingsRow label="Provider name">
          <TextInput v-model="label" placeholder="Anthropic" maxlength="64" class="w-56 max-w-full" />
          <Button :disabled="!label.trim()" @click="add">Add provider</Button>
        </SettingsRow>
      </form>
    </SettingsGroup>
    <SettingsNote v-if="message" :text="message" />
  </SettingsPage>
</template>

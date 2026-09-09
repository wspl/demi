<script setup lang="ts">
import Button from '../ui/Button.vue'
import ShortcutRecorder from '../ui/ShortcutRecorder.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsNote from './SettingsNote.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsKeyBinding } from './types'

/** Every shortcut as a row with a recorder. The host judges a new binding and answers on the note. */
defineProps<{
  bindings: SettingsKeyBinding[]
  /** Why the last change was refused, or empty. */
  message?: string
}>()

const emit = defineEmits<{
  rebind: [id: string, keys: string]
  reset: []
}>()
</script>

<template>
  <SettingsPage
    title="Keyboard"
    description="Click one to change it. The browser keeps some keys for itself."
  >
    <SettingsGroup title="Shortcuts">
      <SettingsRow
        v-for="binding in bindings"
        :key="binding.id"
        :label="binding.action"
      >
        <ShortcutRecorder
          :model-value="binding.keys"
          size="sm"
          @update:model-value="emit('rebind', binding.id, $event)"
        />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup>
      <SettingsRow label="Reset all shortcuts">
        <Button size="sm" @click="emit('reset')">Reset</Button>
      </SettingsRow>
    </SettingsGroup>
    <SettingsNote v-if="message" :text="message" />
  </SettingsPage>
</template>

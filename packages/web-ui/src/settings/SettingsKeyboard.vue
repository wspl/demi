<script setup lang="ts">
import { ref } from 'vue'
import Button from '../ui/Button.vue'
import ShortcutRecorder from '../ui/ShortcutRecorder.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import { shortcutRefusal } from './shortcuts'
import type { SettingsKeyBinding } from './types'

/**
 * Every shortcut as a row with a recorder. Keys that would fire while the
 * user types, or that another action holds, are refused, and the row says
 * why under its name; the shortcut keeps its keys.
 */
const props = defineProps<{
  bindings: SettingsKeyBinding[]
}>()

const emit = defineEmits<{
  rebind: [id: string, keys: string]
  reset: []
}>()

/** The last refused change: whose row says why. */
const refused = ref<{ id: string; reason: string } | null>(null)

function record(id: string, keys: string): void {
  const reason = shortcutRefusal(keys, id, props.bindings)
  if (reason) {
    refused.value = { id, reason }
    return
  }
  refused.value = null
  emit('rebind', id, keys)
}

function reset(): void {
  refused.value = null
  emit('reset')
}
</script>

<template>
  <SettingsPage
    title="Keyboard"
    description="Click Change, then press the new keys with ⌘ or ⌃. The browser keeps some keys for itself."
  >
    <SettingsGroup title="Shortcuts">
      <SettingsRow
        v-for="binding in bindings"
        :key="binding.id"
        :label="binding.action"
      >
        <template v-if="refused?.id === binding.id" #description>
          <span role="status" class="text-on-warning">{{ refused.reason }}</span>
        </template>
        <ShortcutRecorder
          :model-value="binding.keys"
          size="sm"
          @update:model-value="record(binding.id, $event)"
        />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup>
      <SettingsRow label="Reset all shortcuts">
        <Button size="sm" @click="reset">Reset</Button>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>

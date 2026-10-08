<script setup lang="ts">
import CopyCode from '../ui/CopyCode.vue'
import type { SentenceText } from '../ui/ui-text'
import type { DeviceStart } from './installation'

/**
 * How to start an offline device's runner again: a sentence, and the
 * command, with Copy. The command starts the runner in the background and
 * returns once it is connected. The sentence takes the text style of where
 * it stands; the composer's notice says where to type the command, and a
 * device's page header passes its own, shorter `sentence`.
 */
defineProps<{
  start: DeviceStart
  sentence?: SentenceText
}>()
</script>

<template>
  <div class="flex min-w-0 flex-col gap-1.5">
    <p>
      {{
        sentence
          ?? (start.system === 'windows'
            ? 'If its runner stopped, run this in PowerShell on the device.'
            : 'If its runner stopped, run this in a terminal on the device.')
      }}
    </p>
    <CopyCode :code="start.command" copy-label="Copy start command" />
  </div>
</template>

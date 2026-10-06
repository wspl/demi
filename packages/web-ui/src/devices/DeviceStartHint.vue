<script setup lang="ts">
import CopyCode from '../ui/CopyCode.vue'
import type { DeviceStart } from './installation'

/**
 * How to start an offline device's runner again: where to type the command
 * its installation's launcher takes, and the command, with Copy. The runner
 * runs in the terminal it was typed into.
 */
defineProps<{
  start: DeviceStart
}>()
</script>

<template>
  <div class="flex min-w-0 flex-col gap-1.5">
    <p class="text-[12px] leading-4 text-fg-subtle">
      {{
        start.system === 'windows'
          ? 'If its runner stopped, run this in PowerShell on the device. Keep it open.'
          : 'If its runner stopped, run this in a terminal on the device. Keep it open.'
      }}
    </p>
    <CopyCode :code="start.command" copy-label="Copy start command" />
  </div>
</template>

<script setup lang="ts">
import CopyCode from '../ui/CopyCode.vue'
import type { DeviceStart } from './installation'

/**
 * How to start an offline device's runner again: where to type the command,
 * and the command, with Copy. The command starts the runner in the
 * background and returns once it is connected.
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
          ? 'If its runner stopped, run this in PowerShell on the device.'
          : 'If its runner stopped, run this in a terminal on the device.'
      }}
    </p>
    <CopyCode :code="start.command" copy-label="Copy start command" />
  </div>
</template>

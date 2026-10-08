<script setup lang="ts">
import DeviceStartHint from '../devices/DeviceStartHint.vue'
import type { DeviceStart } from '../devices/installation'

/**
 * The conversation's primary Host is a paired device whose runner is not
 * connected: offered above the composer with how to start the runner again,
 * and gone once the device is online.
 */
defineProps<{
  /** The device's name. */
  name: string
  start: DeviceStart
}>()
</script>

<template>
  <div
    class="host-offline-notice flex w-full flex-col gap-2 rounded-lg bg-surface-raised p-3 text-chrome text-fg-muted"
    role="status"
  >
    <span class="min-w-0 break-words">{{ name }} is offline.</span>
    <DeviceStartHint class="text-[12px] leading-4 text-fg-subtle" :start="start" />
  </div>
</template>

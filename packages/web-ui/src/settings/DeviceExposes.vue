<script setup lang="ts">
import DeviceExposeRow from './DeviceExposeRow.vue'
import type { SettingsExpose } from './types'

/**
 * One device's exposes under its row in the devices settings: the address,
 * the time left, the public URL to copy or open, and renew and remove. The
 * host supplies the list and performs the actions; an expired expose stays
 * until the next snapshot drops it.
 */
defineProps<{
  exposes: SettingsExpose[]
  /** Expose ids with a renew or remove request in flight. */
  pendingIds?: string[]
}>()
const emit = defineEmits<{
  renew: [id: string]
  remove: [id: string]
}>()
</script>

<template>
  <div class="flex flex-col">
    <DeviceExposeRow
      v-for="expose in exposes"
      :key="expose.id"
      :expose="expose"
      :pending="pendingIds?.includes(expose.id)"
      @renew="emit('renew', expose.id)"
      @remove="emit('remove', expose.id)"
    />
    <div
      v-if="!exposes.length"
      class="select-none bg-overlay/[0.025] py-1.5 pl-7 pr-4 text-[12px] leading-5 text-fg-subtle"
    >
      No URLs exposed. Create one with
      <code>demi host expose add &lt;address&gt;</code>.
    </div>
  </div>
</template>

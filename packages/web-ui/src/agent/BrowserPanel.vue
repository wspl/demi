<script setup lang="ts">
import { ArrowLeft, ArrowRight, RotateCw } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import TextInput from '../ui/TextInput.vue'
import type { BrowserWorkTab } from './work-panel'

/** Address controls for one local browser tab; Host navigation is not connected yet. */
const props = defineProps<{ tab: BrowserWorkTab }>()
const emit = defineEmits<{ update: [tab: BrowserWorkTab] }>()

function updateAddress(address: string): void {
  emit('update', { ...props.tab, address })
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Back" disabled disabled-reason="Browser navigation is not connected yet" />
      <IconButton :icon="ArrowRight" variant="ghost" aria-label="Forward" disabled disabled-reason="Browser navigation is not connected yet" />
      <IconButton :icon="RotateCw" variant="ghost" aria-label="Refresh" disabled disabled-reason="Browser navigation is not connected yet" />
      <TextInput
        class="ml-2 min-w-0 flex-1"
        :model-value="tab.address"
        placeholder="Enter address"
        aria-label="Browser address"
        @update:model-value="updateAddress"
      />
    </div>
    <div class="flex min-h-0 flex-1 items-center justify-center border-t border-line-subtle text-[13px] text-fg-faint">
      Browser is not connected yet.
    </div>
  </div>
</template>

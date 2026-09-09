<script setup lang="ts">
import { watch } from 'vue'
import { Search } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from './Dialog.vue'
import ScrollArea from './ScrollArea.vue'
import TextInput from './TextInput.vue'
import { ICON_PX } from './icon-metrics'

const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  label: string
}>()
const emit = defineEmits<{ close: [] }>()
const query = defineModel<string>('query', { default: '' })
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      query.value = ''
    }
  },
)
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :label="label"
    :scroll-content="false"
    @close="emit('close')"
  >
    <!-- The dialog caps this stable height to the host viewport. Results never size it. -->
    <div class="flex h-[36rem] min-h-0 flex-col">
      <div class="flex shrink-0 flex-col gap-4 p-5 pb-3">
        <header class="select-none pr-10">
          <h3 class="text-[15px] font-medium text-fg-emphasis">{{ label }}</h3>
        </header>
        <TextInput
          v-model="query"
          placeholder="Search"
          aria-label="Search"
          focused
        >
          <template #prefix><Search :size="ICON_PX.in24" /></template>
        </TextInput>
      </div>
      <ScrollArea
        class="min-h-0 flex-1"
        viewport-class="flex flex-col gap-4 px-5 pb-5 pt-2"
      >
        <slot />
      </ScrollArea>
    </div>
  </Dialog>
</template>

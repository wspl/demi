<script setup lang="ts">
/**
 * A widget whole in a tab of the work panel: its page (`./WidgetFrame.vue`)
 * fills the pane, with no card, and content taller than the pane scrolls
 * inside the page. A widget whose document its route could not serve says so
 * in the pane, as its block does in the reply.
 */
import { ref, watch } from 'vue'
import { SheetCloseButton, useSheetDismiss } from '@demicodes/plugin-sdk'
import { t } from './locales/text'
import type { WidgetTabData } from './tab'
import WidgetFrame from './WidgetFrame.vue'

const props = defineProps<{ conversation: string, tabId: string, data: WidgetTabData, shown: boolean }>()

const unavailable = ref(false)
watch(() => props.data.id, () => {
  unavailable.value = false
})
// A widget has no bar of its own; in a phone overlay it shows a thin one with its title and Close.
const sheet = useSheetDismiss()
</script>

<template>
  <div class="flex size-full flex-col">
    <div v-if="sheet" class="flex h-row shrink-0 select-none items-center gap-1 border-b border-line pl-3 pr-1.5 text-detail text-fg">
      <span class="min-w-0 flex-1 truncate">{{ data.title }}</span>
      <SheetCloseButton />
    </div>
    <div class="relative min-h-0 flex-1">
      <WidgetFrame v-if="!unavailable" :id="data.id" :title="data.title" @unavailable="unavailable = true" />
      <p v-else class="absolute inset-0 m-0 flex items-center justify-center text-fg-muted">{{ t('block.missing', { id: data.id }) }}</p>
    </div>
  </div>
</template>

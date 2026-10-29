<script setup lang="ts">
/**
 * A widget whole in a tab of the work panel: its page (`./WidgetFrame.vue`)
 * fills the pane, with no card, and content taller than the pane scrolls
 * inside the page. A widget whose document its route could not serve says so
 * in the pane, as its block does in the reply.
 */
import { ref, watch } from 'vue'
import { t } from './locales/text'
import type { WidgetTabData } from './tab'
import WidgetFrame from './WidgetFrame.vue'

const props = defineProps<{ conversation: string, tabId: string, data: WidgetTabData, shown: boolean }>()

const unavailable = ref(false)
watch(() => props.data.id, () => {
  unavailable.value = false
})
</script>

<template>
  <div class="relative size-full">
    <WidgetFrame v-if="!unavailable" :id="data.id" :title="data.title" @unavailable="unavailable = true" />
    <p v-else class="absolute inset-0 m-0 flex items-center justify-center text-fg-muted">{{ t('block.missing', { id: data.id }) }}</p>
  </div>
</template>

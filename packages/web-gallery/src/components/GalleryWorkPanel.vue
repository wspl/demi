<script setup lang="ts">
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import type { GalleryWork } from '../fixtures/work-panel'

/** The product's work panel over one specimen's state, every event acting on it as the product's store would. */
defineProps<{ work: GalleryWork; beforeFirstMessage?: boolean }>()
const emit = defineEmits<{ close: [] }>()
</script>

<template>
  <WorkPanel
    :panel="work.panel.value"
    :pinned="work.pinned.value"
    :kinds="work.kinds"
    :before-first-message="beforeFirstMessage"
    @select="work.select"
    @add-tab="work.add"
    @update-tab="work.update"
    @update-pinned="work.updatePinned"
    @close-tabs="work.closeTabs"
    @close="emit('close')"
  />
</template>

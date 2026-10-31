<script setup lang="ts">
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import type { GalleryWork } from '../fixtures/work-panel'

/** The product's work panel over one specimen's state, every event acting on it as the product's store would. */
defineProps<{ work: GalleryWork; beforeFirstMessage?: boolean; hostOffline?: string | null }>()
const emit = defineEmits<{ close: [] }>()
</script>

<template>
  <!-- The panel is a pane of its own, as the product's is (`SidebarLayout`), so a menu stays inside it. -->
  <WorkPanel
    data-region
    :panel="work.panel.value"
    :pinned="work.pinned.value"
    :kinds="work.kinds.value"
    :before-first-message="beforeFirstMessage"
    :host-offline="hostOffline"
    :sheet="work.sheet.value"
    @select="work.select"
    @move-tab="work.move"
    @update-tab="work.update"
    @update-pinned="work.updatePinned"
    @close-tabs="work.closeTabs"
    @close="emit('close')"
  />
</template>

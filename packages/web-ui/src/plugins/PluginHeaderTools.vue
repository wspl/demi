<script setup lang="ts">
import type { PluginPage } from './slots'

/**
 * The conversation header's tools of the shown pages, each of which shows
 * itself only while its plugin has something to show.
 */
defineProps<{
  pages: readonly PluginPage[]
  shown: (page: PluginPage) => boolean
  conversationId: string
  hostName: (id: string) => string
}>()

const emit = defineEmits<{
  openTab: [kind: string, data: unknown]
  manageDevices: []
}>()
</script>

<template>
  <template v-for="page in pages" :key="page.plugin">
    <component
      :is="page.headerTool"
      v-if="page.headerTool && shown(page)"
      :conversation-id="conversationId"
      :host-name="hostName"
      @open-tab="(kind: string, data: unknown) => emit('openTab', kind, data)"
      @manage-devices="emit('manageDevices')"
    />
  </template>
</template>

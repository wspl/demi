<script setup lang="ts">
import type { ToolCallBlock as ToolCall } from '@demicodes/web-ui/agent/block-types'
import ToolCallBlock from '@demicodes/web-ui/agent/blocks/ToolCallBlock.vue'
import ImageViewer from '@demicodes/web-ui/files/ImageViewer.vue'
import { provideImageViewer } from '@demicodes/web-ui/files/image-viewer'

/**
 * Calls whose results carry media, at a phone's width: the page's viewer of
 * its own, so an image opened large fills this host as it fills a phone's
 * screen. Place it in a narrow overlay well.
 */
defineProps<{ calls: readonly ToolCall[] }>()

const viewer = provideImageViewer()
</script>

<template>
  <div class="flex flex-col gap-1 px-3 py-4">
    <ToolCallBlock
      v-for="call in calls"
      :key="call.id"
      :block="call"
      :is-streaming="false"
    />
  </div>
  <ImageViewer :viewer="viewer" />
</template>

<script setup lang="ts">
import CodePreview from '@demicodes/web-ui/ui/CodePreview.vue'
import { galleryState } from '../gallery-state'
import { previewCode } from '../fixtures/previews'
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <header class="shrink-0 border-b border-line px-5 py-3">
      <h2 class="text-[15px] font-medium text-fg-emphasis">CodePreview</h2>
    </header>
    <CodePreview
      :key="galleryState.mode"
      class="min-h-0 flex-1"
      :code="previewCode"
      lang="typescript"
    />
  </div>
</template>

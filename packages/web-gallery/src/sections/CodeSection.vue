<script setup lang="ts">
import CodePreview from '@demicodes/web-ui/ui/CodePreview.vue'
import { galleryState } from '../gallery-state'
import { previewCode } from '../fixtures/previews'
</script>

<template>
  <CodePreview
    :key="galleryState.mode"
    class="min-h-0 flex-1"
    :code="previewCode"
    lang="typescript"
  />
</template>

<script setup lang="ts">
import { ref } from 'vue'
import MarkdownPreview from '@demicodes/web-ui/ui/MarkdownPreview.vue'
import { md } from '@demicodes/web-ui/markdown/md'
import { galleryState } from '../gallery-state'
import { previewMarkdown } from '../fixtures/previews'

const content = ref(previewMarkdown)
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <header class="shrink-0 border-b border-line px-5 py-3">
      <h2 class="text-[15px] font-medium text-fg-emphasis">MarkdownPreview</h2>
    </header>
    <MarkdownPreview
      :key="galleryState.mode"
      v-model:content="content"
      class="min-h-0 flex-1"
      :render-markdown="md.render"
    />
  </div>
</template>

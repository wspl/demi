<script setup lang="ts">
import { ref } from 'vue'
import MarkdownPreview from '@demicodes/web-ui/ui/MarkdownPreview.vue'
import { md } from '@demicodes/web-ui/markdown/md'
import { galleryState } from '../gallery-state'
import { previewMarkdown } from '../fixtures/previews'

const content = ref(previewMarkdown)
</script>

<template>
  <MarkdownPreview
    :key="galleryState.mode"
    v-model:content="content"
    class="min-h-0 flex-1"
    :render-markdown="md.render"
  />
</template>

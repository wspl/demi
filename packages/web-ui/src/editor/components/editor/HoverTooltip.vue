<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  text: string
  renderMarkdown: (text: string) => string
}>()

const html = computed(() => props.renderMarkdown(props.text))
</script>

<template>
  <div
    class="markdown-body max-w-[600px] max-h-[300px] overflow-y-auto overflow-x-hidden p-2 text-xs leading-relaxed text-fg [&_pre]:m-0! [&_pre]:p-1.5! [&_pre]:text-xs! [&_pre]:whitespace-pre-wrap! [&_p]:mt-2! [&_p]:mb-0! [&_p:first-child]:mt-0! [&_code]:text-xs! [&_code]:whitespace-pre-wrap!"
    v-html="html"
  />
</template>

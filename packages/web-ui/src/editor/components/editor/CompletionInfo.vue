<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  detail?: string
  documentation?: string
  renderMarkdown: (text: string) => string
}>()

const html = computed(() => {
  const parts: string[] = []
  if (props.detail) parts.push('```typescript\n' + props.detail + '\n```')
  if (props.documentation) parts.push(props.documentation)
  return props.renderMarkdown(parts.join('\n\n'))
})
</script>

<template>
  <div
    class="markdown-body max-w-[400px] max-h-[250px] overflow-y-auto overflow-x-hidden p-2 text-xs leading-relaxed text-fg [&_pre]:m-0! [&_pre]:p-1.5! [&_pre]:text-xs! [&_pre]:whitespace-pre-wrap! [&_p]:mt-2! [&_p]:mb-0! [&_p:first-child]:mt-0! [&_code]:text-xs! [&_code]:whitespace-pre-wrap! [&>*:last-child]:mb-0!"
    v-html="html"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { MarkdownLinkHandler, MarkdownRenderer } from '../markdown/types'

const props = defineProps<{
  content: string
  basePath?: string
  renderMarkdown: MarkdownRenderer
  onLinkClick?: MarkdownLinkHandler
}>()

const renderedHtml = computed(() =>
  props.renderMarkdown(props.content, props.basePath ? { basePath: props.basePath } : undefined),
)

function handleClick(event: MouseEvent) {
  const target = (event.target as HTMLElement).closest('a')
  if (!target) return
  const href = target.getAttribute('href')
  if (!href) return
  props.onLinkClick?.({ href, event, basePath: props.basePath })
}
</script>

<template>
  <div
    class="markdown-body h-full overflow-y-auto bg-surface-editor p-4 text-sm leading-relaxed text-fg-body select-text"
    v-html="renderedHtml"
    @click="handleClick"
  />
</template>

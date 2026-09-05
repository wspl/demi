<script setup lang="ts">
import { computed } from 'vue'
import type { MarkdownLinkHandler, MarkdownRenderer } from '../markdown/types'
import { toggleGfmTask } from '../markdown/gfm-task'

const content = defineModel<string>('content', { required: true })

const props = defineProps<{
  basePath?: string
  renderMarkdown: MarkdownRenderer
  onLinkClick?: MarkdownLinkHandler
}>()

const renderedHtml = computed(() =>
  props.renderMarkdown(content.value, props.basePath ? { basePath: props.basePath } : undefined),
)

function handleClick(event: MouseEvent) {
  const target = event.target as HTMLElement
  if (target instanceof HTMLInputElement && target.type === 'checkbox') {
    const root = event.currentTarget as HTMLElement
    const index = [...root.querySelectorAll('input[type="checkbox"]')].indexOf(target)
    if (index >= 0) content.value = toggleGfmTask(content.value, index)
    return
  }
  const link = target.closest('a')
  if (!link) return
  const href = link.getAttribute('href')
  if (!href) return
  props.onLinkClick?.({ href, event, basePath: props.basePath })
}
</script>

<template>
  <div
    class="markdown-body h-full overflow-y-auto bg-surface-editor p-4 text-conversation text-fg-body select-text"
    v-html="renderedHtml"
    @click="handleClick"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { md } from '@demi/web-ui/markdown/md'
import { isHttpUrl } from '@demi/web-ui/markdown/filePath'

const props = defineProps<{
  content: string
}>()

const renderedMarkdown = computed(() => md.render(props.content))

function handleClick(event: MouseEvent) {
  const target = (event.target as HTMLElement).closest('a')
  if (!target) return

  const href = target.getAttribute('href')
  if (!href) return

  if (isHttpUrl(href)) {
    event.preventDefault()
    window.open(href, '_blank', 'noopener,noreferrer')
  }
}
</script>

<template>
  <div class="px-8 py-1.5">
    <div
      class="markdown-body select-text text-sm leading-relaxed text-fg-body"
      v-html="renderedMarkdown"
      @click="handleClick"
    />
  </div>
</template>

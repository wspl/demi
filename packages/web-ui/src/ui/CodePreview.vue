<script setup lang="ts">
import { computed } from 'vue'
import { codeToHtml, useMarkdownRenderVersion } from '../markdown/highlight'

const props = withDefaults(defineProps<{
  code: string
  lang?: string
}>(), {
  lang: 'text',
})

const renderVersion = useMarkdownRenderVersion()

const html = computed(() => {
  void renderVersion.value
  return codeToHtml(props.code, props.lang)
})
</script>

<template>
  <div
    class="code-preview markdown-body h-full overflow-y-auto bg-surface-editor p-4 text-conversation select-text"
    v-html="html"
  />
</template>

<style scoped>
.code-preview :deep(pre) {
  margin: 0;
  padding: 0;
  background: transparent;
  border-radius: 0;
}
</style>

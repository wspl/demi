<script setup lang="ts">
import { computed } from 'vue'
import claudeIconRaw from './icons/claude.svg?raw'
import openaiIconRaw from './icons/openai.svg?raw'

const BUILTIN_SVGS: Record<string, string> = {
  'claude-code': claudeIconRaw,
  anthropic: claudeIconRaw,
  codex: openaiIconRaw,
  openai: openaiIconRaw,
}

const props = defineProps<{
  providerId: string
  size?: number
}>()

const s = computed(() => props.size ?? 16)
const svgContent = computed(() => BUILTIN_SVGS[props.providerId] ?? '')
</script>

<template>
  <span class="inline-block shrink-0" :style="{ width: `${s}px`, height: `${s}px` }" v-html="svgContent" />
</template>

<style scoped>
span :deep(svg) {
  width: 100%;
  height: 100%;
  color: inherit;
}
</style>

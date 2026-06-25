<script setup lang="ts">
import { computed } from 'vue'
import { FlashLine } from '@mingcute/vue/flash'
import FunctionalBlock from './FunctionalBlock.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText } from '../block-helpers'
import { trimToolSummary } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
}>()

const summary = computed(() => {
  const entries = Object.entries(props.input)
  if (entries.length === 0) return ''
  return entries
    .map(([k, v]) => {
      const val = typeof v === 'string' ? v : JSON.stringify(v)
      const short = val.length > 40 ? `${val.slice(0, 37)}...` : val
      return `${k}=${short}`
    })
    .join(' ')
})
const errorText = computed(() => getToolErrorText(props.block))
const detail = computed(() => {
  const text = errorText.value
  return text ? trimToolSummary(text, 160) : summary.value
})
</script>

<template>
  <FunctionalBlock
    :label="block.toolName"
    :detail="detail"
    :loading="block.status === 'executing'"
    :error="block.status === 'error'"
  >
    <template #icon>
      <FlashLine :size="16" />
    </template>
  </FunctionalBlock>
</template>

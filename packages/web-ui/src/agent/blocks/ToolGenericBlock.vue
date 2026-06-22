<script setup lang="ts">
import { computed } from 'vue'
import { FlashLine } from '@mingcute/vue/flash'
import CollapsibleBlock from './CollapsibleBlock.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText } from '../block-helpers'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
}>()

const errorText = computed(() => getToolErrorText(props.block))
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
</script>

<template>
  <CollapsibleBlock
    :label="block.toolName"
    :detail="summary"
    :loading="block.status === 'executing'"
    :error="block.status === 'error'"
    :error-text="errorText"
  >
    <template #icon>
      <FlashLine :size="16" />
    </template>
  </CollapsibleBlock>
</template>

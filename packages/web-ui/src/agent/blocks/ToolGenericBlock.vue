<script setup lang="ts">
import { computed } from 'vue'
import { Zap } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
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
    :tone="block.status === 'error' ? 'danger' : undefined"
  >
    <template #icon>
      <Zap :size="ICON_PX.in28" />
    </template>
  </FunctionalBlock>
</template>

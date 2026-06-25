<script setup lang="ts">
import { computed } from 'vue'
import { HistoryLine } from '@mingcute/vue/history'
import { SearchLine } from '@mingcute/vue/search'
import { SendLine } from '@mingcute/vue/send'
import { StopLine } from '@mingcute/vue/stop'
import FunctionalBlock from './FunctionalBlock.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText } from '../block-helpers'
import { standardToolTitle, trimToolSummary, type ControlToolName } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  toolName: ControlToolName
}>()

const title = computed(() => standardToolTitle(props.toolName, props.input))
const errorText = computed(() => getToolErrorText(props.block))
const errorSummary = computed(() => {
  const text = errorText.value
  return text ? trimToolSummary(text, 160) : ''
})
const iconComponent = computed(() => {
  switch (props.toolName) {
    case 'shell_status':
      return SearchLine
    case 'shell_write':
      return SendLine
    case 'shell_abort':
      return StopLine
    case 'yield':
      return HistoryLine
  }
})
</script>

<template>
  <FunctionalBlock
    :loading="block.status === 'executing'"
    :error="block.status === 'error'"
  >
    <template #icon>
      <component :is="iconComponent" :size="16" />
    </template>

    <template #default="{ loading }">
      <span class="min-w-0 truncate" :class="loading ? 'thinking-shimmer' : ''">{{ title }}</span>
      <span
        v-if="errorSummary"
        class="min-w-0 truncate font-mono text-fg-subtle"
      >{{ errorSummary }}</span>
    </template>
  </FunctionalBlock>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { History, SquareTerminal } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
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
    case 'shell_write':
    case 'shell_abort':
      return SquareTerminal
    case 'yield':
      return History
  }
})
</script>

<template>
  <FunctionalBlock
    :loading="block.status === 'executing'"
    :tone="block.status === 'error' ? 'danger' : undefined"
  >
    <template #icon>
      <component :is="iconComponent" :size="ICON_PX.in28" />
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

<script setup lang="ts">
import { computed } from 'vue'
import { parse, Allow } from 'partial-json'
import type { ToolCallBlock } from '../block-types'
import ToolShellBlock from './ToolShellBlock.vue'
import ToolGenericBlock from './ToolGenericBlock.vue'

const STREAMING_INPUT_TOOLS = new Set(['shell_exec'])

const props = defineProps<{
  block: ToolCallBlock
  conversationId: string
  isStreaming: boolean
}>()

const parsedInput = computed<Record<string, unknown>>(() => {
  if (!props.block.input) return {}
  try {
    const result = STREAMING_INPUT_TOOLS.has(props.block.toolName)
      ? parse(props.block.input, Allow.ALL)
      : JSON.parse(props.block.input)
    return typeof result === 'object' && result !== null ? result as Record<string, unknown> : {}
  } catch {
    return {}
  }
})
</script>

<template>
  <ToolShellBlock v-if="block.toolName === 'shell_exec'" :block="block" :input="parsedInput" :is-streaming="isStreaming" />
  <ToolGenericBlock v-else :block="block" :input="parsedInput" />
</template>

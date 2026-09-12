<script setup lang="ts">
import { computed } from 'vue'
import type { ToolCallBlock } from '../block-types'
import { parseToolCallInput } from '../block-helpers'
import ToolShellBlock from './ToolShellBlock.vue'
import ToolShellStatusBlock from './ToolShellStatusBlock.vue'
import ToolShellWriteBlock from './ToolShellWriteBlock.vue'
import ToolShellAbortBlock from './ToolShellAbortBlock.vue'
import ToolYieldBlock from './ToolYieldBlock.vue'
import ToolGenericBlock from './ToolGenericBlock.vue'
import { toolRenderKind } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  conversationId: string
  isStreaming: boolean
}>()

const parsedInput = computed(() => parseToolCallInput(props.block))
const renderKind = computed(() => toolRenderKind(props.block.toolName))
</script>

<template>
  <ToolShellBlock
    v-if="renderKind === 'shell_exec'"
    :block="block"
    :input="parsedInput"
    :is-streaming="isStreaming"
  />
  <ToolShellStatusBlock
    v-else-if="renderKind === 'shell_status'"
    :block="block"
    :input="parsedInput"
  />
  <ToolShellWriteBlock
    v-else-if="renderKind === 'shell_write'"
    :block="block"
    :input="parsedInput"
  />
  <ToolShellAbortBlock
    v-else-if="renderKind === 'shell_abort'"
    :block="block"
    :input="parsedInput"
  />
  <ToolYieldBlock
    v-else-if="renderKind === 'yield'"
    :block="block"
    :input="parsedInput"
  />
  <ToolGenericBlock
    v-else
    :block="block"
    :input="parsedInput"
  />
</template>

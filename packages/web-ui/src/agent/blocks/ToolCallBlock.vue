<script setup lang="ts">
import { computed } from 'vue'
import type { ToolCallBlock } from '../block-types'
import { parseToolCallInput } from '../block-helpers'
import ToolShellBlock from './ToolShellBlock.vue'
import ToolShellStatusBlock from './ToolShellStatusBlock.vue'
import ToolYieldBlock from './ToolYieldBlock.vue'
import ToolGenericBlock from './ToolGenericBlock.vue'
import { toolRenderKind } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
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

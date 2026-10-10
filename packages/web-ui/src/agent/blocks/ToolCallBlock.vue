<script setup lang="ts">
import { computed } from 'vue'
import type { ToolCallBlock } from '../block-types'
import { parseToolCallInput } from '../block-helpers'
import ToolShellBlock from './ToolShellBlock.vue'
import ToolGenericBlock from './ToolGenericBlock.vue'
import { toolRenderKind } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
}>()

const parsedInput = computed(() => parseToolCallInput(props.block))
const renderKind = computed(() => toolRenderKind(props.block.toolName))
</script>

<template>
  <ToolShellBlock
    v-if="renderKind === 'shell'"
    :block="block"
    :input="parsedInput"
  />
  <ToolGenericBlock
    v-else
    :block="block"
    :input="parsedInput"
  />
</template>

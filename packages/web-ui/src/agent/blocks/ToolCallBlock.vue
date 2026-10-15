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
const isOpen = defineModel<boolean>('open', { default: false })

const parsedInput = computed(() => parseToolCallInput(props.block))
const renderKind = computed(() => toolRenderKind(props.block.toolName))
</script>

<template>
  <ToolShellBlock
    v-if="renderKind === 'shell'"
    v-model:open="isOpen"
    :block="block"
    :input="parsedInput"
  />
  <ToolGenericBlock
    v-else
    v-model:open="isOpen"
    :block="block"
  />
</template>

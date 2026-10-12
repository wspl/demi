<script setup lang="ts">
import { computed } from 'vue'
import type { ToolCallBlock } from '../block-types'
import { parseToolCallInput } from '../block-helpers'
import ToolShellBlock from './ToolShellBlock.vue'
import ToolGenericBlock from './ToolGenericBlock.vue'
import { toolRenderKind } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  /** The call is the row of a run the running turn has not moved past: it shimmers whatever the call's state (`runtime.md` § Work groups). */
  working?: boolean
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
    :working="working"
  />
  <ToolGenericBlock
    v-else
    v-model:open="isOpen"
    :block="block"
    :working="working"
  />
</template>

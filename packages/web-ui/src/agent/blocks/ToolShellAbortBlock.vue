<script setup lang="ts">
import type { ToolCallBlock } from '../block-types'
import ToolShellControlBlock from './ToolShellControlBlock.vue'

defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
}>()
</script>

<template>
  <ToolShellControlBlock :block="block" :input="input" tool-name="shell_abort" />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import RunEndTag from './RunEndTag.vue'
import ShellRowTag from './ShellRowTag.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText, parseToolCallInput } from '../block-helpers'
import { shellTitle, toolRenderKind, trimToolSummary } from '../tool-rendering'

/**
 * What a call's row says on its line (`runtime.md` § Rendering boundary),
 * wherever the row stands: alone, among an opened run's steps, or as the
 * folded face of the run whose newest step it is (§ Work groups). A
 * `shell` call reads its title and its command's tag; a call of another
 * tool its name and its input, or its failure, in short, marked Failed
 * when it failed.
 */
const props = defineProps<{ block: ToolCallBlock }>()

const input = computed(() => parseToolCallInput(props.block))
const shell = computed(() => toolRenderKind(props.block.toolName) === 'shell')
/** Another tool's input in short, or what its failure said. */
const detail = computed(() => {
  const failure = getToolErrorText(props.block)
  if (failure)
    return trimToolSummary(failure, 160)
  return Object.entries(input.value)
    .map(([key, value]) => {
      const text = typeof value === 'string' ? value : JSON.stringify(value)
      return `${key}=${text.length > 40 ? `${text.slice(0, 37)}...` : text}`
    })
    .join(' ')
})
</script>

<template>
  <template v-if="shell">
    <span class="min-w-0 truncate">{{ shellTitle(input) }}</span>
    <ShellRowTag :block="block" />
  </template>
  <template v-else>
    <span class="shrink-0">{{ block.toolName }}</span>
    <span v-if="detail" class="min-w-0 truncate font-mono text-fg-muted group-hover:text-fg-body">{{ detail }}</span>
    <RunEndTag v-if="block.status === 'error'" kind="failed" />
  </template>
</template>

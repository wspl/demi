<script setup lang="ts">
import { computed } from 'vue'
import { TerminalBoxLine } from '@mingcute/vue/terminal-box'
import AnsiText from './AnsiText.vue'
import CollapsibleBlock from './CollapsibleBlock.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText } from '../block-helpers'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  isStreaming: boolean
}>()

const command = computed(() => (props.input['script'] as string) ?? '')
const description = computed(() => (props.input['description'] as string) ?? '')
// Collapsed, the row exposes only the terminal icon + this title (the model's description, or the
// command itself when none was given). Expanding reveals the actual command and its output.
const title = computed(() => description.value || command.value)
const errorText = computed(() => getToolErrorText(props.block))

const outputText = computed(() => {
  const source =
    props.block.status === 'executing'
      ? props.block.streamingOutput
      : props.block.streamingOutput.length > 0
        ? props.block.streamingOutput
        : props.block.output
  return source.filter((b) => b.type === 'text').map((b) => b.text).join('\n')
})
</script>

<template>
  <CollapsibleBlock
    :loading="block.status === 'executing'"
    :error="block.status === 'error'"
    :error-text="errorText"
  >
    <template #icon>
      <TerminalBoxLine :size="16" />
    </template>

    <template #default="{ loading }">
      <span class="min-w-0 truncate" :class="loading ? 'thinking-shimmer' : ''">{{ title }}</span>
    </template>

    <template #body>
      <div class="flex px-3 py-1 font-mono text-xs">
        <span class="mr-1 shrink-0 select-none text-fg-faint">$</span><span class="min-w-0 select-all whitespace-pre-wrap break-words text-fg-muted">{{ command }}</span>
      </div>
      <AnsiText v-if="outputText" :content="outputText" class="px-3 pb-1" />
    </template>
  </CollapsibleBlock>
</template>

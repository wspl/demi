<script setup lang="ts">
import { computed } from 'vue'
import { TerminalBoxLine } from '@mingcute/vue/terminal-box'
import AnsiText from './AnsiText.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText } from '../block-helpers'
import { standardToolTitle } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  isStreaming: boolean
}>()

const command = computed(() => (props.input['script'] as string) ?? '')
const title = computed(() => standardToolTitle('shell_exec', props.input))
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
  <FunctionalBlock
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
  </FunctionalBlock>
</template>

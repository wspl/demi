<script setup lang="ts">
import { computed } from 'vue'
import { TerminalBoxLine } from '@mingcute/vue/terminal-box'
import AnsiText from './AnsiText.vue'
import ToolCard from './ToolCard.vue'
import type { ToolCallBlock } from '../block-types'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  isStreaming: boolean
}>()

const command = computed(() => (props.input['script'] as string) ?? '')
const description = computed(() => (props.input['description'] as string) ?? '')
const hasCommand = computed(() => !!command.value)

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
  <ToolCard
    :loading="block.status === 'executing'"
    :error="block.status === 'error'"
    :show-body="hasCommand"
    collapsed-height="100px"
  >
    <template #icon>
      <TerminalBoxLine :size="16" />
    </template>

    <template #header>
      <span class="truncate text-[13px] text-fg-muted">{{ description || command }}</span>
    </template>

    <template #body-top>
      <div class="border-b border-line bg-overlay/2 px-3 py-1.5 font-mono text-xs">
        <span class="select-none text-fg-faint">$ </span><span class="line-clamp-5 select-all text-fg-muted">{{ command }}</span>
      </div>
    </template>

    <template #body>
      <AnsiText :content="outputText" class="px-3 py-1.5" />
    </template>
  </ToolCard>
</template>

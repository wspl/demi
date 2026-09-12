<script setup lang="ts">
import { computed } from 'vue'
import { asString } from '@demicodes/utils'
import { SquareTerminal } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import AnsiText from './AnsiText.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText, shellTerminalOutputChunks } from '../block-helpers'
import { standardToolTitle } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  isStreaming: boolean
}>()

const command = computed(() => asString(props.input['script']) ?? '')
const title = computed(() => standardToolTitle('shell_exec', props.input))
const errorText = computed(() => getToolErrorText(props.block))
const terminalOutputText = computed(
  () => shellTerminalOutputChunks(props.block).map((chunk) => chunk.text).join('')
)
const isOpen = defineModel<boolean>('open', { default: false })
</script>

<template>
  <FunctionalBlock
    v-model:open="isOpen"
    :open-while="block.status === 'executing' || isStreaming"
    :loading="block.status === 'executing'"
    :tone="block.status === 'error' ? 'danger' : undefined"
    :error-text="errorText"
    :stick-bottom="block.status === 'executing' || isStreaming"
  >
    <template #icon>
      <SquareTerminal :size="ICON_PX.in28" />
    </template>

    <template #default="{ loading }">
      <span class="min-w-0 truncate" :class="loading ? 'thinking-shimmer' : ''">{{ title }}</span>
    </template>

    <template #body>
      <div
        class="mx-3 my-1 space-y-1 rounded-md border border-line-subtle bg-surface-base px-3 py-2"
      >
        <div class="flex font-mono text-xs leading-5 text-fg-subtle">
          <span class="mr-1 shrink-0 select-none text-fg-faint">$</span><span
            class="min-w-0 select-all whitespace-pre-wrap break-words"
          >{{ command }}</span>
        </div>
        <AnsiText v-if="terminalOutputText" :content="terminalOutputText" />
      </div>
    </template>
  </FunctionalBlock>
</template>

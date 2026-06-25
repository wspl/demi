<script setup lang="ts">
import { computed } from 'vue'
import { HistoryLine } from '@mingcute/vue/history'
import { SearchLine } from '@mingcute/vue/search'
import { SendLine } from '@mingcute/vue/send'
import { StopLine } from '@mingcute/vue/stop'
import AnsiText from './AnsiText.vue'
import CollapsibleBlock from './CollapsibleBlock.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText, toolOutputText } from '../block-helpers'
import { standardToolRows, standardToolTitle, type ControlToolName } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
  toolName: ControlToolName
}>()

const title = computed(() => standardToolTitle(props.toolName, props.input))
const rows = computed(() => standardToolRows(props.toolName, props.input))
const errorText = computed(() => getToolErrorText(props.block))
const outputText = computed(() => toolOutputText(props.block))
const hasBody = computed(() => rows.value.length > 0 || outputText.value.length > 0)
const iconComponent = computed(() => {
  switch (props.toolName) {
    case 'shell_status':
      return SearchLine
    case 'shell_write':
      return SendLine
    case 'shell_abort':
      return StopLine
    case 'yield':
      return HistoryLine
  }
})
</script>

<template>
  <CollapsibleBlock
    :loading="block.status === 'executing'"
    :error="block.status === 'error'"
    :error-text="errorText"
  >
    <template #icon>
      <component :is="iconComponent" :size="16" />
    </template>

    <template #default="{ loading }">
      <span class="min-w-0 truncate" :class="loading ? 'thinking-shimmer' : ''">{{ title }}</span>
    </template>

    <template v-if="hasBody" #body>
      <div v-if="rows.length > 0" class="space-y-1 px-3 py-1.5 text-xs">
        <div v-for="row in rows" :key="row.label" class="grid grid-cols-[88px_minmax(0,1fr)] gap-2">
          <span class="text-fg-faint">{{ row.label }}</span>
          <span
            class="min-w-0 break-words text-fg-muted"
            :class="row.monospace ? 'font-mono select-all' : ''"
          >{{ row.value }}</span>
        </div>
      </div>
      <AnsiText v-if="outputText" :content="outputText" class="px-3 pb-1" />
    </template>
  </CollapsibleBlock>
</template>

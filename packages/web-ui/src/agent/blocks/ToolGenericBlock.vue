<script setup lang="ts">
import { computed } from 'vue'
import { Zap } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from './FunctionalBlock.vue'
import ToolMedia from './ToolMedia.vue'
import type { ToolCallBlock } from '../block-types'
import { getToolErrorText } from '../block-helpers'
import { takesMediumPlace } from '../tool-media'
import { trimToolSummary } from '../tool-rendering'

const props = defineProps<{
  block: ToolCallBlock
  input: Record<string, unknown>
}>()

const summary = computed(() => {
  const entries = Object.entries(props.input)
  if (entries.length === 0) {
    return ''
  }
  return entries
    .map(([k, v]) => {
      const val = typeof v === 'string' ? v : JSON.stringify(v)
      const short = val.length > 40 ? `${val.slice(0, 37)}...` : val
      return `${k}=${short}`
    })
    .join(' ')
})
// The result's own text; its media, and a text in a medium's place, show under the row.
const texts = computed(() =>
  props.block.output.flatMap((part) => part.type === 'text' && !takesMediumPlace(part) ? [part.text] : []),
)
const errorText = computed(() => getToolErrorText(props.block))
const detail = computed(() => {
  const text = errorText.value
  return text ? trimToolSummary(text, 160) : summary.value
})
</script>

<template>
  <FunctionalBlock
    :label="block.toolName"
    :detail="detail"
    :loading="block.status === 'executing'"
    :tone="block.status === 'error' ? 'danger' : undefined"
  >
    <template
      v-if="texts.length > 0"
      #body
    >
      <div class="space-y-2 py-2">
        <pre
          v-for="(text, index) in texts"
          :key="index"
          class="whitespace-pre-wrap break-words text-chrome"
          >{{ text }}</pre
        >
      </div>
    </template>
    <template #icon>
      <Zap :size="ICON_PX.in28" />
    </template>
  </FunctionalBlock>
  <ToolMedia :output="block.output" :title="block.toolName" />
</template>

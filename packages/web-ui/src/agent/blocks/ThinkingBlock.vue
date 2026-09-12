<script setup lang="ts">
import { computed } from 'vue'
import { Brain } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import { thinkingFaceLabel } from '../thinking-label'
import { useElapsedTime } from '../../composables/useElapsedTime'

const props = defineProps<{
  thinking: string
  isStreaming: boolean
  createdAt: string
  /** Start of the block after this one — the moment thinking ended. Null while still thinking. */
  endedAt?: string | null
}>()

const hasContent = computed(() => props.thinking.trim().length > 0)
const isOpen = defineModel<boolean>('open', { default: false })

const elapsedMs = useElapsedTime(
  () => Date.parse(props.createdAt),
  () => props.isStreaming,
  () => props.endedAt ? Date.parse(props.endedAt) : null,
)
const label = computed(() => thinkingFaceLabel(props.isStreaming, elapsedMs.value))
const rollKey = computed(() => (props.isStreaming ? 'live' : 'done'))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <FunctionalBlock
      v-model:open="isOpen"
      :expandable="hasContent"
      :open-while="isStreaming && hasContent"
      :stick-bottom="isStreaming"
      :roll-key="rollKey"
    >
      <template #icon>
        <Brain :size="ICON_PX.in28" />
      </template>
      <span
        class="min-w-0 truncate"
        :class="isStreaming ? 'thinking-shimmer' : ''"
      >{{ label }}</span>
      <template v-if="hasContent" #body>
        <StreamedMarkdown
          :content="thinking"
          :streaming="isStreaming"
          class="px-3 py-1 text-[13px] leading-5 text-fg-subtle"
        />
      </template>
    </FunctionalBlock>
  </div>
</template>

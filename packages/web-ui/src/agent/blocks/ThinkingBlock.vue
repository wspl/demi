<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { Brain } from '@lucide/vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import FunctionalBlock from './FunctionalBlock.vue'
import { thinkingFaceLabel } from '../thinking-label'

const props = defineProps<{
  thinking: string
  isStreaming: boolean
  createdAt: string
  /** Start of the block after this one — the moment thinking ended. Null while still thinking. */
  endedAt?: string | null
}>()

const hasContent = computed(() => props.thinking.trim().length > 0)
const isOpen = ref(false)

// Live timer while thinking; once done the elapsed is frozen to (next block's createdAt - this
// block's createdAt), so the duration survives reload instead of growing from the original time.
const nowMs = ref(Date.now())
let timer: ReturnType<typeof setInterval> | undefined
function stopTimer() {
  if (timer) {
    clearInterval(timer)
    timer = undefined
  }
}
watch(
  () => props.isStreaming,
  (streaming) => {
    stopTimer()
    if (streaming) {
      nowMs.value = Date.now()
      timer = setInterval(() => {
        nowMs.value = Date.now()
      }, 1000)
    }
  },
  { immediate: true },
)
onUnmounted(stopTimer)

const startMs = computed(() => Date.parse(props.createdAt))
const elapsedMs = computed(() => {
  const end = props.endedAt ? Date.parse(props.endedAt) : props.isStreaming ? nowMs.value : null
  if (end === null || Number.isNaN(startMs.value)) return null
  return Math.max(0, end - startMs.value)
})
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
      <span class="min-w-0 truncate" :class="isStreaming ? 'thinking-shimmer' : ''">{{ label }}</span>
      <template v-if="hasContent" #body>
        <StreamedMarkdown
          :content="thinking"
          :streaming="isStreaming"
          class="px-3 py-1 text-conversation text-fg-muted"
        />
      </template>
    </FunctionalBlock>
  </div>
</template>

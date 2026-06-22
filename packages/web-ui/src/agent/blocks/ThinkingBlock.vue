<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { BrainLine } from '@mingcute/vue/brain'
import { md } from '@demi/web-ui/markdown/md'
import { t } from '@demi/web-ui/infra/i18n'
import { useThinkingDisclosure } from '@demi/web-ui/composables/useThinkingDisclosure'
import CollapsibleBlock from './CollapsibleBlock.vue'

const props = defineProps<{
  thinking: string
  isStreaming: boolean
  createdAt: string
  /** Start of the block after this one — the moment thinking ended. Null while still thinking. */
  endedAt?: string | null
}>()

const hasContent = computed(() => props.thinking.trim().length > 0)
const isOpen = useThinkingDisclosure({
  hasContent: () => hasContent.value,
  isStreaming: () => props.isStreaming,
})
const renderedMarkdown = computed(() => md.render(props.thinking))

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
const label = computed(() => {
  if (elapsedMs.value === null) return t('agent.block.thinking')
  const prefix = t(props.isStreaming ? 'agent.block.thinkingFor' : 'agent.block.thoughtFor')
  return `${prefix} ${formatDuration(elapsedMs.value)}`
})

function formatDuration(ms: number): string {
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  const rs = s % 60
  if (m < 60) return rs ? `${m}m${rs}s` : `${m}m`
  const h = Math.floor(m / 60)
  const rm = m % 60
  return rm ? `${h}h${rm}m` : `${h}h`
}
</script>

<template>
  <div class="px-8">
    <CollapsibleBlock v-model:open="isOpen" :expandable="hasContent" :stick-bottom="isStreaming">
      <template #icon>
        <BrainLine :size="16" />
      </template>
      <span class="min-w-0 truncate" :class="isStreaming ? 'thinking-shimmer' : ''">{{ label }}</span>
      <template v-if="hasContent" #body>
        <div class="markdown-body px-3 py-1 text-[13px] leading-relaxed text-fg-muted" v-html="renderedMarkdown" />
      </template>
    </CollapsibleBlock>
  </div>
</template>

<style scoped>
/* While the model is actively thinking (this block is the last one and the turn is running) we
   shimmer the label so it reads as a live "thinking" indicator — whether or not prose has streamed
   in yet — and once thinking ends it goes static. */
.thinking-shimmer {
  background: linear-gradient(
    100deg,
    rgb(105 105 105) 0%,
    rgb(105 105 105) 38%,
    rgb(250 250 250) 50%,
    rgb(105 105 105) 62%,
    rgb(105 105 105) 100%
  );
  background-size: 200% 100%;
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: thinking-shimmer 1.6s ease-in-out infinite;
}

@keyframes thinking-shimmer {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}
</style>

<script setup lang="ts">
import { computed } from 'vue'
import { Brain, SquareTerminal, Zap } from '@lucide/vue'
import type { MessageListBlock } from '@demicodes/web-ui/agent/pending-steers'
import { thinkingFaceLabel } from '@demicodes/web-ui/agent/thinking-label'
import { standardToolTitle, toolRenderKind } from '@demicodes/web-ui/agent/tool-rendering'
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { ActivityKind } from '../turn-flow'

const props = defineProps<{
  kind: ActivityKind
  label: string
  /** The block rolling into this row: the slot shows its 28px face until the transcript takes it. */
  incoming?: MessageListBlock | null
}>()

const faceKey = computed(() => props.incoming?.id ?? props.kind)
const iconKey = computed(() => (props.incoming ? `block:${props.incoming.id}` : 'sweep'))

const incomingIcon = computed(() => {
  const block = props.incoming
  if (block?.type === 'thinking') return Brain
  if (block?.type === 'tool_call') return toolRenderKind(block.toolName) === 'shell_exec' ? SquareTerminal : Zap
  return null
})

const incomingLabel = computed(() => {
  const block = props.incoming
  if (block?.type === 'thinking') return thinkingFaceLabel(true, null)
  if (block?.type === 'tool_call') {
    const kind = toolRenderKind(block.toolName)
    return kind === 'generic' ? block.toolName : standardToolTitle(kind, JSON.parse(block.input || '{}'))
  }
  return ''
})
</script>

<template>
  <div class="flex h-7 items-center px-[var(--agent-pad-x,2rem)] text-chrome text-fg-muted">
    <ChromeRoll :face-key="faceKey" :icon-key="iconKey">
      <template #icon>
        <span
          class="flex h-7 shrink-0 items-center justify-center"
          :style="{ width: `${ICON_PX.in28}px`, height: `${ICON_PX.in28}px` }"
        >
          <component :is="incomingIcon" v-if="incomingIcon" :size="ICON_PX.in28" />
          <ActivityMark v-else />
        </span>
      </template>
      <span class="min-w-0 truncate thinking-shimmer">{{ incoming ? incomingLabel : label }}</span>
    </ChromeRoll>
  </div>
</template>

<script setup lang="ts">
import { computed, type Component } from 'vue'
import { Brain, History, SquareTerminal } from '@lucide/vue'
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { t } from '@demicodes/web-ui/infra/i18n'
import type { ActivityKind, HandoffBlock } from '../activity-slot'
import { parseToolCallInput } from '../block-helpers'
import { thinkingFaceLabel } from '../thinking-label'
import { standardToolTitle, toolRenderKind } from '../tool-rendering'

/**
 * The transcript's tail row while it waits: the wait mark and why. A block
 * handed to it rolls in with the face its transcript row will have, so the
 * row that takes over afterwards looks the same.
 */
const props = defineProps<{
  kind: ActivityKind
  incoming?: HandoffBlock | null
}>()

const faceKey = computed(() => props.incoming?.id ?? props.kind)
// The wait mark stays while only the reason changes; a block brings its own icon, so the whole face rolls.
const iconKey = computed(() => (props.incoming ? `block:${props.incoming.id}` : 'wait'))

const waitLabel = computed(() => {
  switch (props.kind) {
    case 'connecting':
      return t('agent.block.connecting')
    case 'resuming':
      return t('agent.block.resuming')
    case 'retrying':
      return t('agent.block.retrying')
    case 'requesting':
      return t('agent.block.requesting')
  }
})

/** Icon and label of the block's own transcript row: ThinkingBlock, ToolShellBlock, ToolShellControlBlock or ToolGenericBlock. */
const incomingFace = computed<{ icon: Component | null; label: string } | null>(() => {
  const block = props.incoming
  if (!block) {
    return null
  }
  if (block.type === 'thinking') {
    return { icon: Brain, label: thinkingFaceLabel(true, null) }
  }
  const kind = toolRenderKind(block.toolName)
  if (kind === 'generic') {
    return { icon: null, label: block.toolName }
  }
  const label = standardToolTitle(kind, parseToolCallInput(block))
  return { icon: kind === 'yield' ? History : SquareTerminal, label }
})
const label = computed(() => incomingFace.value?.label ?? waitLabel.value)
</script>

<template>
  <div class="flex h-7 items-center gap-2 px-[var(--agent-pad-x,2rem)] text-chrome text-fg-muted">
    <ChromeRoll
      class="min-w-0"
      :face-key="faceKey"
      :icon-key="iconKey"
    >
      <template #icon>
        <span
          v-if="!incomingFace || incomingFace.icon"
          class="flex shrink-0 items-center justify-center"
          :style="{ width: `${ICON_PX.in28}px`, height: `${ICON_PX.in28}px` }"
        >
          <component
            :is="incomingFace.icon"
            v-if="incomingFace?.icon"
            :size="ICON_PX.in28"
          />
          <ActivityMark v-else />
        </span>
      </template>
      <span class="min-w-0 truncate thinking-shimmer">{{ label }}</span>
    </ChromeRoll>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch, type Component } from 'vue'
import { Brain, History, SquareTerminal } from '@lucide/vue'
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { t } from '@demicodes/web-ui/infra/i18n'
import type { ActivityKind, HandoffBlock } from '../activity-slot'
import { parseToolCallInput } from '../block-helpers'
import { useElapsedTime } from '../../composables/useElapsedTime'
import { formatThinkingDuration, thinkingFaceLabel } from '../thinking-label'
import { standardToolTitle, toolRenderKind } from '../tool-rendering'
import FunctionalBlock from './FunctionalBlock.vue'

/**
 * The transcript's tail row while it waits: a FunctionalBlock face with the
 * wait mark and why. A block handed to it rolls in with the icon and label its
 * own row (ThinkingBlock, ToolShellBlock, ToolShellControlBlock or
 * ToolGenericBlock) will have, so that row takes over without a change.
 */
const props = defineProps<{
  kind: ActivityKind
  incoming?: HandoffBlock | null
}>()

const isRequesting = computed(() => props.kind === 'requesting' && !props.incoming)
const requestingSince = ref(Date.now())
watch(isRequesting, (requesting) => {
  if (requesting) {
    requestingSince.value = Date.now()
  }
})
const requestingElapsed = useElapsedTime(
  () => requestingSince.value,
  () => isRequesting.value,
)

interface Face {
  /** `wait` is the ActivityMark; null is a row without an icon cell. */
  icon: Component | 'wait' | null
  label: string
}

const waitLabel = computed(() => {
  switch (props.kind) {
    case 'connecting':
      return t('agent.block.connecting')
    case 'resuming':
      return t('agent.block.resuming')
    case 'retrying':
      return t('agent.block.retrying')
    case 'requesting':
      return `${t('agent.block.requestingFor')} ${formatThinkingDuration(requestingElapsed.value ?? 0)}`
  }
})

const face = computed<Face>(() => {
  const block = props.incoming
  if (!block) {
    return { icon: 'wait', label: waitLabel.value }
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

const rollKey = computed(() => props.incoming?.id ?? props.kind)
// The wait mark stays while only the reason changes; a block brings its own icon, so the whole face rolls.
const iconKey = computed(() => (props.incoming ? `block:${props.incoming.id}` : 'wait'))
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)]">
    <FunctionalBlock
      loading
      :roll-key="rollKey"
      :icon-key="iconKey"
    >
      <template v-if="face.icon !== null" #icon>
        <ActivityMark v-if="face.icon === 'wait'" />
        <component
          :is="face.icon"
          v-else
          :size="ICON_PX.in28"
        />
      </template>
      <span class="min-w-0 truncate thinking-shimmer">{{ face.label }}</span>
    </FunctionalBlock>
  </div>
</template>

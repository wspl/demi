<script setup lang="ts">
import { computed, type Component } from 'vue'
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { ActivityKind, HandoffBlock } from '../activity-slot'
import { toolCallTitle } from '../block-helpers'
import { useElapsedTime } from '../../composables/useElapsedTime'
import { providerWaitLabel, thinkingFaceLabel } from '../thinking-label'
import { pendingCallTitle } from '../tool-rendering'
import FunctionalBlock from './FunctionalBlock.vue'
import NoticeStepFace from './NoticeStepFace.vue'
import { stepIcon } from './tool-row-icon'
import { isNoticeStep, type NoticeStep } from '../work-groups'

/**
 * The transcript's tail row while it waits: a FunctionalBlock face with the
 * wait mark and why. A block handed to it rolls in with the icon and label its
 * own row (ThinkingBlock, ToolShellBlock, ToolGenericBlock or a notice row)
 * will have, so that row takes over without a change.
 */
const props = defineProps<{
  kind: ActivityKind
  incoming?: HandoffBlock | null
  /** When the wait began, as `Date.now()`; Requesting shows how long it has waited since. */
  since: number
}>()

const requestingElapsed = useElapsedTime(
  () => props.since,
  () => props.kind === 'requesting' && !props.incoming,
)

interface Face {
  /** `wait` is the ActivityMark; null is a row without an icon cell. */
  icon: Component | 'wait' | null
  label: string
  /** A notice or a receipt rolling in, which the face says as its own row does. */
  notice: NoticeStep | null
}

const waitLabel = computed(() => {
  switch (props.kind) {
    case 'connecting':
      return 'Connecting'
    case 'requesting':
      return providerWaitLabel(requestingElapsed.value ?? 0)
  }
})

const face = computed<Face>(() => {
  const block = props.incoming
  if (!block) {
    return { icon: 'wait', label: waitLabel.value, notice: null }
  }
  const { icon } = stepIcon(block)
  if (isNoticeStep(block)) {
    return { icon, label: '', notice: block }
  }
  switch (block.type) {
    case 'thinking':
      return { icon, label: thinkingFaceLabel(true, null), notice: null }
    case 'pending_call':
      return { icon, label: pendingCallTitle(block.call), notice: null }
    case 'tool_call':
      return { icon, label: toolCallTitle(block), notice: null }
  }
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
      <NoticeStepFace v-if="face.notice" :step="face.notice" />
      <span v-else class="min-w-0 truncate">{{ face.label }}</span>
    </FunctionalBlock>
  </div>
</template>

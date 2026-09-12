<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useScroll } from '@vueuse/core'
import type { QueuedMessage, SessionPhase } from '@demicodes/core'
import type { DisplayedBlock as Block } from '@demicodes/agent/client'
import { BLOCK_GAP, useBlockVirtualizer, type PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import { getVisibleBlocks } from './visible-blocks'
import { isTextBlockStreaming, isThinkingBlockStreaming } from './block-streaming'
import { pendingSteersToRenderBlocks, type MessageListBlock } from './pending-steers'
import { queuedMessagesToRenderBlocks } from './queued-messages'
import { activitySlotKind, type PendingAction } from './activity-slot'
import { useActivityHandoff } from './useActivityHandoff'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import AgentMessageVirtualBlock from './blocks/AgentMessageVirtualBlock.vue'
import ActivitySlot from './blocks/ActivitySlot.vue'
import SessionStatus from './SessionStatus.vue'
import ErrorNotice from '../ui/ErrorNotice.vue'
import {
  sessionPaneStatus,
  type SessionFailureNotice,
  type SessionLoad,
} from './session-status'
import { COMPOSER_CLEARANCE_PX } from './composer-clearance'
import { t } from '../infra/i18n'
import MessageEditRegion from './MessageEditRegion.vue'
import { lastEditableUserMessageId, messageEditSuffixIds } from './message-editing'
import { useMessageForks, type MessageForkHandler } from './message-fork'
import { useFollowSentMessages } from './useFollowSentMessages'

const props = defineProps<{
  conversationId: string
  blocks: Block[]
  pendingSteers: PendingSteerMessage[]
  queue: QueuedMessage[]
  phase: SessionPhase
  bottomOffset: number
  persistedScrollState: PersistedScrollState | undefined
  /** Hide editing actions on user bubbles. */
  readOnly?: boolean
  fork?: MessageForkHandler
  editTargetId?: string
  /** History restore. `loading` never reads as an empty conversation. */
  load?: SessionLoad
  /** A recovery the server has not acknowledged: the tail row says Resuming or Retrying. */
  pendingAction?: PendingAction
  loadError?: string | null
  /** A session-level failure told at the tail of the transcript, in flow. */
  failure?: SessionFailureNotice | null
  /** Offered on the error record that ended an idle conversation. */
  retry?: () => void
  pendingSubmission?: PendingSubmissionState | null
}>()

const emit = defineEmits<{
  saveScrollState: [
    conversationId: string,
    state: PersistedScrollState | undefined
  ]
  deletePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  deleteQueued: [id: string]
  sendQueued: [id: string]
  editUser: [blockId: string]
  retryLoad: []
  retrySubmission: []
}>()

const { states: forkStates, run: forkMessage } = useMessageForks(() => props.fork, () => props.conversationId)

const visibleBlocks = computed(() => getVisibleBlocks(props.blocks))
// A recovery hides the record it recovers from: the tail row names the recovery, then the turn running.
const transcriptBlocks = computed(() => {
  const visible = visibleBlocks.value
  const recovering = props.phase !== 'idle' || props.pendingAction === 'resume'
  if (recovering && visible.at(-1)?.type === 'error') {
    return visible.slice(0, -1)
  }
  return visible
})
const editableUserId = computed(() => lastEditableUserMessageId(props.blocks))
const tailBlocks = computed<MessageListBlock[]>(() => [
  ...pendingSteersToRenderBlocks(props.pendingSteers),
  ...queuedMessagesToRenderBlocks(props.queue),
  ...(props.pendingSubmission ? [{
    type: 'pending_submission' as const,
    id: `pending-submission:${props.pendingSubmission.id}`,
    submission: props.pendingSubmission,
  }] : []),
])
const slotKind = computed(() => activitySlotKind({
  load: props.load ?? 'ready',
  phase: props.phase,
  pendingAction: props.pendingAction ?? null,
  transcriptBlocks: visibleBlocks.value,
  renderBlocks: [...transcriptBlocks.value, ...tailBlocks.value],
}))
const { heldId, slot } = useActivityHandoff(
  () => transcriptBlocks.value.at(-1),
  () => slotKind.value,
  () => props.conversationId,
)
// The block rolling into the tail row is not a list row yet.
const visibleTranscriptBlocks = computed(() => {
  const blocks = transcriptBlocks.value
  return heldId.value !== null && blocks.at(-1)?.id === heldId.value
    ? blocks.slice(0, -1)
    : blocks
})
const renderBlocks = computed<MessageListBlock[]>(() => [
  ...visibleTranscriptBlocks.value,
  ...tailBlocks.value,
])

const paneStatus = computed(() =>
  props.pendingSubmission ? null : sessionPaneStatus(props.load ?? 'ready', renderBlocks.value.length > 0),
)
const mutedIds = computed(() => messageEditSuffixIds(renderBlocks.value, props.editTargetId))
// Only the record that ended the conversation is a place to retry from; older
// errors are history.
const retryTargetId = computed(() => {
  const tail = transcriptBlocks.value.at(-1)
  return props.phase === 'idle' && tail?.type === 'error' ? tail.id : null
})

// Every streamed delta re-renders the visible rows; the lookup must not rescan the transcript per row.
const transcriptIndexById = computed(
  () =>
    new Map(visibleTranscriptBlocks.value.map((block, index) => [block.id, index]))
)

function transcriptIndexAt(index: number): number {
  const block = renderBlocks.value[index]
  return block ? transcriptIndexById.value.get(block.id) ?? -1 : -1
}

function isStreamingThinkingAt(index: number): boolean {
  return isThinkingBlockStreaming(
    visibleTranscriptBlocks.value,
    props.phase,
    transcriptIndexAt(index)
  )
}

function isStreamingTextAt(index: number): boolean {
  return isTextBlockStreaming(
    visibleTranscriptBlocks.value,
    props.phase,
    transcriptIndexAt(index)
  )
}

// The next block's createdAt marks when a thinking block stopped (null while it's still the last,
// i.e. actively thinking). Lets ThinkingBlock show a frozen "thought for Xs" that survives reload.
function thinkingEndedAt(index: number): string | null {
  const transcriptIndex = transcriptIndexAt(index)
  if (transcriptIndex < 0)
    return null
  const next = visibleTranscriptBlocks.value[transcriptIndex + 1]
  return next && 'createdAt' in next ? next.createdAt : null
}

const scrollContainer = ref<HTMLDivElement>()

const {
  virtualItems,
  totalSize,
  measureElement,
  scrollOffset,
  isAtBottom,
  scrollToBottom,
  onScroll,
  getPersistedState
} =
  useBlockVirtualizer(scrollContainer, renderBlocks, props.persistedScrollState)

onBeforeUnmount(() => {
  emit('saveScrollState', props.conversationId, getPersistedState())
})

const { isScrolling } = useScroll(scrollContainer, { idle: 1500 })

useFollowSentMessages(() => renderBlocks.value, scrollToBottom)

// Composer or task-control growth covers the tail; a reader at the bottom stays there.
// bottomOffset includes the permanent control row, even when its scroll button is hidden.
watch(
  () => props.bottomOffset,
  () => {
    const wasAtBottom = isAtBottom.value
    nextTick(() => {
      scrollOffset.value = scrollContainer.value?.scrollTop ?? 0
      if (wasAtBottom)
        scrollToBottom()
    })
  },
  { flush: 'post' },
)

defineExpose({
  isAtBottom,
  scrollToBottom,
})
</script>

<template>
  <div class="relative h-full">
    <div
      ref="scrollContainer"
      class="h-full overflow-y-auto"
      :class="[isScrolling ? 'scrollbar-active' : '', paneStatus ? 'flex flex-col' : '']"
      style="overflow-anchor: none; scrollbar-gutter: stable;"
      @scroll="onScroll"
    >
      <SessionStatus
        v-if="paneStatus"
        :kind="paneStatus"
        :detail="paneStatus === 'failed' ? (loadError ?? undefined) : undefined"
        @retry="emit('retryLoad')"
      />
      <div
        v-else
        class="w-full pt-2"
        :style="{ paddingBottom: `${props.bottomOffset + COMPOSER_CLEARANCE_PX}px` }"
      >
        <div class="relative w-full" :style="{ height: `${totalSize}px` }">
          <div
            v-for="item in virtualItems"
            :key="String(item.key)"
            :data-index="item.index"
            :ref="(el) => measureElement(el as Element)"
            class="absolute inset-x-0 top-0"
            :style="{ transform: `translateY(${item.start}px)` }"
          >
            <MessageEditRegion :muted="mutedIds.has(renderBlocks[item.index]!.id)">
              <AgentMessageVirtualBlock
                :block="renderBlocks[item.index]!"
                :conversation-id="props.conversationId"
                :is-thinking-streaming="isStreamingThinkingAt(item.index)"
                :is-text-streaming="isStreamingTextAt(item.index)"
                :thinking-ended-at="thinkingEndedAt(item.index)"
                :fork="fork ? () => forkMessage(renderBlocks[item.index]!.id) : undefined"
                :fork-state="forkStates.get(renderBlocks[item.index]!.id)"
                :retry="renderBlocks[item.index]!.id === retryTargetId ? retry : undefined"
                :editable="renderBlocks[item.index]!.id === editableUserId && !props.readOnly && phase === 'idle' && !queue.length && !pendingSteers.length"
                @delete-pending-steer="(id) => emit('deletePendingSteer', id)"
                @interrupt-pending-steer="(id) => emit('interruptPendingSteer', id)"
                @delete-queued="(id) => emit('deleteQueued', id)"
                @send-queued="(id) => emit('sendQueued', id)"
                @edit-user="(id) => emit('editUser', id)"
                @retry-submission="emit('retrySubmission')"
              />
            </MessageEditRegion>
          </div>
        </div>
        <ActivitySlot
          v-if="slot"
          :kind="slot.kind"
          :incoming="slot.incoming"
          :style="{ marginTop: renderBlocks.length ? `${BLOCK_GAP}px` : '0' }"
        />
        <div v-if="failure" class="px-[var(--agent-pad-x,2rem)] py-1.5">
          <ErrorNotice
            :label="failure.label"
            :action="failure.retry ? t('agent.session.retry') : undefined"
            @action="emit('retryLoad')"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useScroll } from '@vueuse/core'
import type { Block, QueuedMessage, SessionPhase } from '@demicodes/core'
import { useBlockVirtualizer, type PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import { getVisibleBlocks } from './visible-blocks'
import { isTextBlockStreaming, isThinkingBlockStreaming } from './block-streaming'
import { pendingSteersToRenderBlocks, type MessageListBlock } from './pending-steers'
import { queuedMessagesToRenderBlocks } from './queued-messages'
import { shouldShowTailLoading } from './tail-loading'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import AgentMessageVirtualBlock from './blocks/AgentMessageVirtualBlock.vue'
import LoadingBlock from './blocks/LoadingBlock.vue'
import SessionStatus from './SessionStatus.vue'
import { sessionPaneStatus, sessionShowsReconnectTail, type SessionLoad } from './session-status'
import { COMPOSER_CLEARANCE_PX } from './composer-clearance'
import { t } from '../infra/i18n'
import MessageEditRegion from './MessageEditRegion.vue'
import { lastEditableUserMessageId, messageEditSuffixIds } from './message-editing'
import { useMessageForks, type MessageForkHandler } from './message-fork'

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
  loadError?: string | null
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

const visibleTranscriptBlocks = computed(() => getVisibleBlocks(props.blocks))
const editableUserId = computed(() => lastEditableUserMessageId(props.blocks))
const renderBlocks = computed<MessageListBlock[]>(() => [
  ...visibleTranscriptBlocks.value,
  ...pendingSteersToRenderBlocks(props.pendingSteers),
  ...queuedMessagesToRenderBlocks(props.queue),
  ...(props.pendingSubmission ? [{
    type: 'pending_submission' as const,
    id: `pending-submission:${props.pendingSubmission.id}`,
    submission: props.pendingSubmission,
  }] : []),
])

const paneStatus = computed(() =>
  props.pendingSubmission ? null : sessionPaneStatus(props.load ?? 'ready', renderBlocks.value.length > 0),
)
const mutedIds = computed(() => messageEditSuffixIds(renderBlocks.value, props.editTargetId))
const reconnecting = computed(() => sessionShowsReconnectTail(props.load ?? 'ready'))
const shouldShowLoading = computed(() =>
  reconnecting.value || shouldShowTailLoading(
    props.phase,
    visibleTranscriptBlocks.value,
    renderBlocks.value
  ),
)
const tailLabel = computed(() => (reconnecting.value
  ? t('agent.block.connecting')
  : undefined))

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

// Composer or task-control growth covers the tail; a reader at the bottom stays there.
// The floating scroll button does not contribute to bottomOffset.
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
        <LoadingBlock v-if="shouldShowLoading" :label="tailLabel" />
      </div>
    </div>
  </div>
</template>

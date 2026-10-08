<script setup lang="ts">
import type { ProviderFailureFacts } from '@demicodes/protocol'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import type { Block, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { BLOCK_GAP, useBlockVirtualizer, type PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import { compactionSummaryTokens, getVisibleBlocks } from './visible-blocks'
import { assistantFooterIds } from './assistant-footer'
import { isTextBlockStreaming, isThinkingBlockStreaming } from './block-streaming'
import type { MessageListBlock } from './pending-steers'
import { listTailBlocks } from './list-tail'
import type { PendingAction } from './activity-slot'
import { useActivitySlot } from './useActivitySlot'
import { useChromeEntrance } from './useChromeEntrance'
import { chromeEntrance } from '../ui/chrome-enter'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import AgentMessageVirtualBlock from './blocks/AgentMessageVirtualBlock.vue'
import ActivitySlot from './blocks/ActivitySlot.vue'
import SessionStatus from './SessionStatus.vue'
import ErrorNotice from '../ui/ErrorNotice.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import {
  sessionPaneStatus,
  type SessionFailureNotice,
  type SessionLoad,
} from './session-status'
import { COMPOSER_CLEARANCE_PX } from './composer-clearance'
import MessageEditRegion from './MessageEditRegion.vue'
import { messageEditSuffixIds, offeredEditId } from './message-editing'
import { useMessageForks, type MessageForkHandler } from './message-fork'
import { useFollowSentMessages } from './useFollowSentMessages'
import { highlightFound } from '../ui/found-highlight'
import RequestChangesLine from './blocks/RequestChangesLine.vue'
import { provideTranscript, useEditSelection } from './edit-selection'
import { requestLineSelection, transcriptRequests, type TranscriptRequest } from '../files/request-changes'

const props = defineProps<{
  conversationId: string
  /** The agent whose transcript this is: null for the conversation's own, a subagent's id otherwise. */
  node?: string | null
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
  /** The host's connection banner says the backend is away; a reconnecting socket adds no Connecting row. */
  backendAway?: boolean
  /** A recovery the server has not acknowledged: the tail row says Requesting. */
  pendingAction?: PendingAction
  /** What the providers read out of the error blocks' failure records, by block id. */
  failures?: Record<string, ProviderFailureFacts>
  loadError?: string | null
  /** A session-level failure told at the tail of the transcript, in flow. */
  failure?: SessionFailureNotice | null
  pendingSubmission?: PendingSubmissionState | null
  /** A block to bring into view and mark for a moment, as a search result opened at it asks; once shown, `revealed` says so. */
  revealBlockId?: string | null
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
  /** Regenerate on the last answer: a new answer to the message `blockId`. */
  regenerate: [blockId: string]
  retryLoad: []
  retrySubmission: []
  revealed: []
}>()

const { states: forkStates, run: forkMessage } = useMessageForks(() => props.fork, () => props.conversationId)

const visibleBlocks = computed(() => getVisibleBlocks(props.blocks))
const footerIds = computed(() => assistantFooterIds(visibleBlocks.value, props.phase))
// A recovery hides the record it recovers from: the tail row names the recovery, then the turn running.
const transcriptBlocks = computed(() => {
  const visible = visibleBlocks.value
  const recovering = props.phase !== 'idle' || props.pendingAction === 'resume'
  if (recovering && visible.at(-1)?.type === 'error') {
    return visible.slice(0, -1)
  }
  return visible
})
const editableUserId = computed(() => props.readOnly ? null : offeredEditId(props))
/**
 * The answer whose footer offers Regenerate: the last one after the message
 * the page offers to edit, since regenerating is that edit
 * (`message-editing.md` § Regenerate and the Up Arrow key).
 */
const regenerateId = computed(() => {
  const target = editableUserId.value
  if (target === null) {
    return null
  }
  const blocks = visibleBlocks.value
  const after = blocks.findIndex((block) => block.id === target)
  return blocks.findLast((block, index) => index > after && footerIds.value.has(block.id))?.id ?? null
})

function regenerate(): void {
  if (editableUserId.value !== null) {
    emit('regenerate', editableUserId.value)
  }
}
const tailBlocks = computed(() => listTailBlocks({
  phase: props.phase,
  pendingSteers: props.pendingSteers,
  queue: props.queue,
  pendingSubmission: props.pendingSubmission,
}))
// The summary size each compaction divider tells, by the id of the block that shows it.
const summaryTokens = computed(() => compactionSummaryTokens(props.blocks))
const slotInput = computed(() => ({
  load: props.load ?? 'ready',
  backendAway: props.backendAway,
  phase: props.phase,
  pendingAction: props.pendingAction ?? null,
  transcriptBlocks: visibleBlocks.value,
  renderBlocks: [...transcriptBlocks.value, ...tailBlocks.value],
}))
const { heldId, slot } = useActivitySlot({
  input: () => slotInput.value,
  pendingSubmission: () => props.pendingSubmission ?? null,
  tail: () => transcriptBlocks.value.at(-1),
  scope: () => props.conversationId,
})
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
const { isEntering } = useChromeEntrance(
  () => visibleTranscriptBlocks.value,
  () => heldId.value,
  () => props.conversationId,
  () => props.load ?? 'ready',
)

const paneStatus = computed(() =>
  props.pendingSubmission ? null : sessionPaneStatus(props.load ?? 'ready', renderBlocks.value.length > 0),
)
const mutedIds = computed(() => messageEditSuffixIds(renderBlocks.value, props.editTargetId))
// Every streamed delta re-renders the visible rows; the lookup must not rescan the transcript per row.
// The transcript's requests, and the line at the end of each that changed files: under the last of its rows.
const requests = computed(() => transcriptRequests(props.blocks))
provideTranscript({
  get node() {
    return props.node ?? null
  },
  requests: () => requests.value,
})
const editSelection = useEditSelection()
const requestLines = computed(() => {
  const last = new Map<TranscriptRequest, string>()
  for (const block of visibleTranscriptBlocks.value) {
    const request = requests.value.requestOf.get(block.id)
    if (request && request.files.length > 0) {
      last.set(request, block.id)
    }
  }
  return new Map([...last].map(([request, id]) => [id, request]))
})

function openRequest(request: TranscriptRequest): void {
  const selection = requestLineSelection(props.node ?? null, request)
  if (selection) {
    editSelection()?.(selection)
  }
}
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

const scrollArea = ref<InstanceType<typeof ScrollArea>>()
const scrollContainer = computed(() => scrollArea.value?.el)

const {
  virtualItems,
  totalSize,
  measureElement,
  scrollOffset,
  isAtBottom,
  scrollToBottom,
  reveal,
  onScroll,
  getPersistedState
} =
  useBlockVirtualizer(scrollContainer, renderBlocks, props.persistedScrollState)

onBeforeUnmount(() => {
  emit('saveScrollState', props.conversationId, getPersistedState())
})

// A block a search result opened: shown once the history holds it, after the list placed itself,
// and marked as it appears; the next request or leaving the list ends a mark still waiting.
let revealing: AbortController | null = null
watch(
  [() => props.revealBlockId, () => renderBlocks.value.length, scrollContainer],
  ([id]) => {
    if (!id)
      return
    void nextTick(() => {
      const scroller = scrollContainer.value
      if (props.revealBlockId !== id || !scroller || !reveal(id))
        return
      revealing?.abort()
      revealing = new AbortController()
      highlightFound(scroller, `[data-block-id="${CSS.escape(id)}"]`, revealing.signal)
      emit('revealed')
    })
  },
  { immediate: true, flush: 'post' },
)
onBeforeUnmount(() => revealing?.abort())

// The part of the transcript the composer leaves visible, which caps a
// message's image height (`file-previews.md` § Files named in messages). Until
// the first measurement the stylesheet falls back to the window's height.
const { height: viewportHeight } = useElementSize(scrollContainer)
const visibleHeightStyle = computed(() => viewportHeight.value > 0
  ? { '--conversation-visible-height': `${Math.max(0, viewportHeight.value - props.bottomOffset)}px` }
  : {})

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
    <ScrollArea
      ref="scrollArea"
      class="h-full"
      :viewport-class="paneStatus ? '[overflow-anchor:none] flex flex-col' : '[overflow-anchor:none]'"
      :style="visibleHeightStyle"
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
        class="w-full px-[var(--session-gutter,0px)] pt-2"
        :style="{ paddingBottom: `${props.bottomOffset + COMPOSER_CLEARANCE_PX}px` }"
      >
        <div class="relative w-full" :style="{ height: `${totalSize}px` }">
          <div
            v-for="item in virtualItems"
            :key="String(item.key)"
            :data-index="item.index"
            :data-block-id="renderBlocks[item.index]!.id"
            :ref="(el) => measureElement(el as Element)"
            class="absolute inset-x-0 top-0"
            :style="{ transform: `translateY(${item.start}px)` }"
          >
            <MessageEditRegion :muted="mutedIds.has(renderBlocks[item.index]!.id)">
              <AgentMessageVirtualBlock
                :block="renderBlocks[item.index]!"
                :is-thinking-streaming="isStreamingThinkingAt(item.index)"
                :is-text-streaming="isStreamingTextAt(item.index)"
                :show-assistant-footer="footerIds.has(renderBlocks[item.index]!.id)"
                :thinking-ended-at="thinkingEndedAt(item.index)"
                :fork="fork ? () => forkMessage(renderBlocks[item.index]!.id) : undefined"
                :fork-state="forkStates.get(renderBlocks[item.index]!.id)"
                :failure="failures?.[renderBlocks[item.index]!.id]"
                :summary-tokens="summaryTokens.get(renderBlocks[item.index]!.id)"
                :entering="isEntering(renderBlocks[item.index]!.id)"
                :editable="renderBlocks[item.index]!.id === editableUserId"
                :regenerate="renderBlocks[item.index]!.id === regenerateId ? regenerate : undefined"
                @delete-pending-steer="(id) => emit('deletePendingSteer', id)"
                @interrupt-pending-steer="(id) => emit('interruptPendingSteer', id)"
                @delete-queued="(id) => emit('deleteQueued', id)"
                @send-queued="(id) => emit('sendQueued', id)"
                @edit-user="(id) => emit('editUser', id)"
                @retry-submission="emit('retrySubmission')"
              >
                <!-- A request's reply ends with its changed-files button, above Copy and Fork. -->
                <template v-if="renderBlocks[item.index]!.type === 'text' && requestLines.has(renderBlocks[item.index]!.id)" #replyEnd>
                  <RequestChangesLine
                    :request="requestLines.get(renderBlocks[item.index]!.id)!"
                    :selectable="editSelection() !== undefined"
                    @open="openRequest(requestLines.get(renderBlocks[item.index]!.id)!)"
                  />
                </template>
              </AgentMessageVirtualBlock>
              <!-- A request that ends on anything else, as while a call still runs, has the button under its last row. -->
              <div
                v-if="renderBlocks[item.index]!.type !== 'text' && requestLines.has(renderBlocks[item.index]!.id)"
                class="px-[var(--agent-pad-x,2rem)] py-1"
              >
                <RequestChangesLine
                  :request="requestLines.get(renderBlocks[item.index]!.id)!"
                  :selectable="editSelection() !== undefined"
                  @open="openRequest(requestLines.get(renderBlocks[item.index]!.id)!)"
                />
              </div>
            </MessageEditRegion>
          </div>
        </div>
        <ActivitySlot
          v-if="slot"
          :key="conversationId"
          v-bind="chromeEntrance(true)"
          :kind="slot.kind"
          :incoming="slot.incoming"
          :since="slot.since"
          :style="{ marginTop: renderBlocks.length ? `${BLOCK_GAP}px` : '0' }"
        />
        <div v-if="failure" class="px-[var(--agent-pad-x,2rem)] py-1.5">
          <ErrorNotice
            :label="failure.label"
            :action="failure.retry ? 'Retry' : undefined"
            @action="emit('retryLoad')"
          />
        </div>
      </div>
    </ScrollArea>
  </div>
</template>

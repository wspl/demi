<script setup lang="ts">
import type { ProviderFailureFacts } from '@demicodes/protocol'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import type { Block, PendingCall, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { BLOCK_GAP, useBlockVirtualizer, type PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import { compactionSummaryTokens, getVisibleBlocks } from './visible-blocks'
import { assistantFooterIds, replyEndIds } from './assistant-footer'
import { isTextBlockStreaming, isThinkingBlockStreaming } from './block-streaming'
import type { MessageListBlock } from './pending-steers'
import { listTailBlocks } from './list-tail'
import { groupWork } from './work-groups'
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
import { provideBlockJump } from './block-jump'
import { commandCalls, provideCommandReferences, useCommandReferences } from './command-references'
import { toolCallTitle } from './block-helpers'
import { provideTranscript } from './edit-selection'
import { transcriptRequests } from '../files/request-changes'

const props = defineProps<{
  conversationId: string
  /** The agent whose transcript this is: null for the conversation's own, a subagent's id otherwise. */
  node?: string | null
  blocks: Block[]
  /** The calls the model is writing, shown after the transcript. */
  pendingCalls?: PendingCall[]
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
/** The blocks that end their reply, after which its work is done: Copy and Fork wait for one. */
const replyEnds = computed(() => replyEndIds(visibleBlocks.value, props.phase))
const footerIds = computed(() => assistantFooterIds(visibleBlocks.value, replyEnds.value))
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
  blocks: props.blocks,
  pendingCalls: props.pendingCalls ?? [],
  pendingSteers: props.pendingSteers,
  queue: props.queue,
  pendingSubmission: props.pendingSubmission,
}))
// The calls the model is writing are the newest steps of the work: they join
// its group and, as any step, roll into the tail row first.
const pendingCallRows = computed(() => tailBlocks.value.filter((block) => block.type === 'pending_call'))
const otherTailBlocks = computed(() => tailBlocks.value.filter((block) => block.type !== 'pending_call'))
const workBlocks = computed<MessageListBlock[]>(() => [...transcriptBlocks.value, ...pendingCallRows.value])
// The summary size each compaction divider tells, by the id of the block that shows it.
const summaryTokens = computed(() => compactionSummaryTokens(props.blocks))
const slotInput = computed(() => ({
  load: props.load ?? 'ready',
  backendAway: props.backendAway,
  phase: props.phase,
  pendingAction: props.pendingAction ?? null,
  transcriptBlocks: visibleBlocks.value,
  renderBlocks: [...groupWork(workBlocks.value, props.phase === 'running'), ...otherTailBlocks.value],
}))
const { heldId, slot } = useActivitySlot({
  input: () => slotInput.value,
  pendingSubmission: () => props.pendingSubmission ?? null,
  tail: () => workBlocks.value.at(-1),
  scope: () => props.conversationId,
})
// The block rolling into the tail row is not a list row yet.
const visibleTranscriptBlocks = computed(() => {
  const blocks = transcriptBlocks.value
  return heldId.value !== null && blocks.at(-1)?.id === heldId.value
    ? blocks.slice(0, -1)
    : blocks
})
const visibleWorkBlocks = computed(() => {
  const blocks = workBlocks.value
  return heldId.value !== null && blocks.at(-1)?.id === heldId.value
    ? blocks.slice(0, -1)
    : blocks
})
const renderBlocks = computed<MessageListBlock[]>(() => [
  ...groupWork(visibleWorkBlocks.value, props.phase === 'running'),
  ...otherTailBlocks.value,
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
/** The row that shows each block: a step's work group, or the block's own. */
const rowOf = computed(() => new Map(renderBlocks.value.flatMap((row) =>
  row.type === 'work_group' ? row.steps.map((step) => [step.id, row.id] as const) : [[row.id, row.id] as const])))
/** Brings the block `id`'s row into view and marks it for a moment; false when the list does not hold it. */
function revealAndMark(id: string): boolean {
  const scroller = scrollContainer.value
  const row = rowOf.value.get(id) ?? id
  if (!scroller || !reveal(row))
    return false
  revealing?.abort()
  revealing = new AbortController()
  highlightFound(scroller, `[data-block-id="${CSS.escape(row)}"]`, revealing.signal)
  return true
}
watch(
  [() => props.revealBlockId, () => renderBlocks.value.length, scrollContainer],
  ([id]) => {
    if (!id)
      return
    void nextTick(() => {
      if (props.revealBlockId === id && revealAndMark(id))
        emit('revealed')
    })
  },
  { immediate: true, flush: 'post' },
)
onBeforeUnmount(() => revealing?.abort())
// A row's reference to another block of this list jumps to it, as a chat
// app jumps to a quoted message.
provideBlockJump((id) => {
  revealAndMark(id)
})
// A look or a wait names a command of this transcript by its call, which
// the reference jumps to, or one of another agent's by its title alone.
const outerReferences = useCommandReferences()
const ownCommands = computed(() => commandCalls(visibleTranscriptBlocks.value))
provideCommandReferences((commandId) => {
  const call = ownCommands.value.get(commandId)
  if (call)
    return { title: toolCallTitle(call), blockId: call.id }
  const outer = outerReferences(commandId)
  return outer && { title: outer.title }
})

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
              </AgentMessageVirtualBlock>
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

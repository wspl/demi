<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import type { ProviderFailureFacts } from '@demicodes/protocol'
import { computed, nextTick, onBeforeUnmount, onScopeDispose, ref, watch } from 'vue'
import { useElementSize, useResizeObserver } from '@vueuse/core'
import type { PendingCall, QueuedMessage, SessionPhase } from '@demicodes/protocol'
import { useBlockVirtualizer, type PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import { compactionSummaryTokens } from './visible-blocks'
import { isTextBlockStreaming, isThinkingBlockStreaming } from './block-streaming'
import { rowKey } from './row-key'
import type { PendingAction } from './activity-slot'
import { useMessageRows } from './message-rows'
import { useLiveCalls } from './live-calls'
import { useChromeEntrance } from './useChromeEntrance'
import type { PendingSteerMessage, PendingSubmissionState } from './types'
import AgentMessageVirtualBlock from './blocks/AgentMessageVirtualBlock.vue'
import SessionStatus from './SessionStatus.vue'
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
import type { AnswerShare } from './sharing'
import { useFollowSentMessages } from './useFollowSentMessages'
import { provideCommandRevealer, useCommandRevealer } from './command-reports'
import { provideOriginRevealer, useOriginRevealer } from './agent-receipts'
import { storedShellView } from './block-helpers'
import { distanceFromBottom } from '../composables/scroll-bottom'
import { highlightFound } from '../ui/found-highlight'
import { provideTranscript } from './edit-selection'
import { useOpenRuns } from './open-runs'
import { followedBlockId } from './work-groups'
import { storedRequests } from '../files/request-changes'
import { blockDirectories, blockMessageFiles } from './working-directory'
import { useMessageFiles } from '../markdown/message-files'
import { useMediaAddress } from './media-source'
import { transcriptPictures } from './transcript-pictures'
import { useMediaViewer } from '../files/media-viewer'
import { versionPlaces, type VersionChoice } from './versions'
import type { HeldBlock } from '@demicodes/conversation-client'
import type { StepsNotSharedRow } from './pending-steers'
import ReplyComments from './comments/ReplyComments.vue'
import { sentComments, type ReplyCommenting } from './comments/comments'
import type { ReplyAsking } from './comments/asks'
import { provideReplySelectionTools } from './reply-selection'

const props = withDefaults(defineProps<{
  conversationId: string
  /**
   * The agent whose transcript this is: null for the conversation's own, a
   * subagent's id otherwise.
   */
  node?: string | null
  /**
   * A subagent's working directory now, which its relative paths read against;
   * absent for the conversation's own agent.
   */
  cwd?: string
  blocks: HeldBlock[]
  /** The calls the model is writing, shown after the transcript. */
  pendingCalls?: PendingCall[]
  /**
   * When the agent began to wait for the model's answer, as the server tells
   * it. The Requesting clock counts from it; null when the agent waits for
   * none.
   */
  waitingSince?: number | null
  pendingSteers: PendingSteerMessage[]
  queue: QueuedMessage[]
  /** The message the running turn started with, until the turn writes it. */
  starting?: QueuedMessage | null
  phase: SessionPhase
  bottomOffset: number
  persistedScrollState: PersistedScrollState | undefined
  /** Hide editing actions on user bubbles. */
  readOnly?: boolean
  fork?: MessageForkHandler
  /**
   * Shares the answer the text block `blockId` ends; absent, no footer offers
   * Share.
   */
  share?: (blockId: string) => void
  /**
   * No answer offers Fork, as on a share's page, which acts on no conversation.
   */
  hideFork?: boolean
  editTargetId?: string
  /** History restore. `loading` never reads as an empty conversation. */
  load?: SessionLoad
  /**
   * The host's connection banner says the backend is away; a reconnecting
   * socket adds no Connecting row.
   */
  backendAway?: boolean
  /**
   * A recovery the server has not acknowledged: the tail row says Requesting.
   */
  pendingAction?: PendingAction
  /**
   * What the providers read out of the error blocks' failure records, by block
   * id.
   */
  failures?: Record<string, ProviderFailureFacts>
  loadError?: ShownError | null
  /** A session-level failure told at the tail of the transcript, in flow. */
  failure?: SessionFailureNotice | null
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSubmissions?: readonly PendingSubmissionState[]
  /**
   * The waiting messages whose Recall is on its way, by id, which show nowhere.
   */
  removing?: readonly string[]
  /**
   * A block to bring into view and mark for a moment, as a search result opened
   * at it asks; once shown, `revealed` says so.
   */
  revealBlockId?: string | null
  /**
   * Where `blocks`, the window the list shows, stands in its transcript.
   *
   * The window reaches the start or the end of the transcript, or both, or
   * neither. It is the latest window or not. Only the latest window shows the
   * turn's tail rows and offers to edit. A whole transcript, as the gallery
   * gives, reaches both ends and is the latest.
   */
  atStart?: boolean
  atEnd?: boolean
  latest?: boolean
  /**
   * Each compaction marker's boundary's summary size, by the marker's id, for a
   * boundary the window does not hold.
   */
  summaries?: Record<string, number>
  /**
   * A share's runs of steps it does not hold, each a line after the row it
   * follows.
   */
  stepsNotShared?: readonly StepsNotSharedRow[]
  /**
   * The comments waiting in the conversation's draft and what to do with them.
   * The replies take comments and show the marks of these and of the ones its
   * messages sent. Absent, as on a share's page or in a subagent's window, the
   * replies take none and show no mark.
   */
  commenting?: ReplyCommenting
  /**
   * The transcript's side questions and what to do with them. The replies take
   * questions and show each one's words, card and fold. Absent, they take none.
   */
  asking?: ReplyAsking
}>(), {
  // Vue reads an absent boolean prop as false; a list given no window is a
  // whole transcript.
  atStart: true,
  atEnd: true,
  latest: true,
})

const emit = defineEmits<{
  saveScrollState: [
    conversationId: string,
    state: PersistedScrollState | undefined
  ]
  recallSteer: [id: string]
  interruptPendingSteer: [id: string]
  recallQueued: [id: string]
  sendQueued: [id: string]
  editUser: [blockId: string]
  /** Regenerate on the last answer: a new answer to the message `blockId`. */
  regenerate: [blockId: string]
  /** Show the `to`th version at the place of the block `blockId`. */
  switchVersion: [blockId: string, to: number]
  retryLoad: []
  /**
   * Edit on the message `id`, whose delivery failed: it goes back into the
   * composer.
   */
  editSubmission: [id: string]
  /** Delete on the message `id`, whose delivery failed: it goes at once. */
  deleteSubmission: [id: string]
  revealed: []
  /**
   * The reader nears the start of the window, which does not reach the
   * transcript's.
   */
  readBefore: []
  /**
   * The reader nears the end of the window, which does not reach the
   * transcript's.
   */
  readAfter: []
}>()

const { states: forkStates, run: forkMessage } = useMessageForks(() => props.fork, () => props.conversationId)

/** Where the rows stand, which the comments' marks are drawn over. */
const rowsRoot = ref<HTMLElement | null>(null)
const comments = ref<InstanceType<typeof ReplyComments>>()
/**
 * The comments the messages sent, those still on their way or waiting for the
 * agent included, which mark their replies dimmer.
 */
const sent = computed(() => [
  ...props.blocks.flatMap(sentComments),
  ...[...props.pendingSteers, ...props.queue, ...(props.pendingSubmissions ?? [])].flatMap((message) => message.comments ?? []),
])
// A reply's selection offers Comment while the list takes comments.
provideReplySelectionTools({
  selection: (reply, streaming) => comments.value?.selectionTools(reply, streaming) ?? [],
  picture: (reply, image, streaming) => comments.value?.pictureTools(reply, image, streaming) ?? [],
})

/**
 * Share on the answer the text block `blockId` ends, which its footer's button
 * starts.
 */
function answerShare(blockId: string): AnswerShare {
  return { start: () => props.share!(blockId) }
}

const {
  visibleBlocks,
  transcriptRows: visibleTranscriptBlocks,
  rows: renderBlocks,
  keyOf,
  footerIds,
} = useMessageRows(() => props, useLiveCalls())
const editableUserId = computed(() => props.readOnly || !props.latest ? null : offeredEditId(props))
/**
 * The answer whose footer offers Regenerate. It is the last answer after the
 * message the page offers to edit, since regenerating is that edit.
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

/**
 * The versions each row shows, by its id: an edited message's under it, a
 * regenerated answer's in its footer.
 *
 * A subagent's transcript shows none, since only the conversation's own path
 * switches. Switching waits, as an edit does, in these cases:
 *
 * - The agent works.
 * - Anything waits to be sent.
 * - The page offers no edit (`readOnly`).
 */
const versionChoices = computed(() => {
  const choices = new Map<string, VersionChoice>()
  if (props.node) {
    return choices
  }
  const busy = props.readOnly || props.phase !== 'idle' || props.queue.length > 0 ||
    props.pendingSteers.length > 0 || props.editTargetId !== undefined
  for (const [rowId, place] of versionPlaces(props.blocks, footerIds.value)) {
    choices.set(rowId, {
      index: place.index,
      count: place.count,
      disabled: busy,
      choose: (to) => emit('switchVersion', place.blockId, to),
    })
  }
  return choices
})
// The summary size each compaction divider tells, by the id of the block that
// shows it.
const summaryTokens = computed(() => new Map([
  ...Object.entries(props.summaries ?? {}),
  ...compactionSummaryTokens(props.blocks),
]))
// A row enters once, as it first appears, under the key the list draws it
// under. A run's row does not enter again as its first step becomes a block,
// nor does a row that takes another's place.
const { isEntering } = useChromeEntrance(
  () => renderBlocks.value.map(keyOf),
  () => props.conversationId,
  () => props.load ?? 'ready',
)

// The tail row alone is no transcript: it waits for one; nor is a failure told
// at the tail.
const paneStatus = computed(() =>
  props.pendingSubmissions?.length ? null : sessionPaneStatus(props.load ?? 'ready', renderBlocks.value.some((row) => row.type !== 'activity_slot' && row.type !== 'session_failure')),
)
const mutedIds = computed(() => messageEditSuffixIds(renderBlocks.value, props.editTargetId))
// The transcript's requests, which its rows' file pills read for what each
// opens. Every streamed delta re-renders the visible rows, so the lookup must
// not rescan the transcript per row.
const requests = computed(() => storedRequests(props.blocks))
// Where a subagent wrote each block, from the working directory blocks of its
// transcript.
const directories = computed(() => props.cwd === undefined ? null : blockDirectories(props.blocks, props.cwd))
const conversationFiles = useMessageFiles()
const mediaAddress = useMediaAddress()
// The pictures the media viewer steps through, read only while one is open.
const pictures = computed(() => transcriptPictures(renderBlocks.value, {
  node: props.node ?? null,
  files: (blockId) => blockMessageFiles(conversationFiles(), directories.value?.get(blockId) ?? props.cwd ?? null, props.node ?? null),
  address: mediaAddress,
}))
// The conversation's own transcript offers its pictures to the page's viewer,
// in which a Media tab restored after a reload finds the one it showed.
if ((props.node ?? null) === null)
  onScopeDispose(useMediaViewer().offer(() => pictures.value))
provideTranscript({
  get node() {
    return props.node ?? null
  },
  requests: () => requests.value,
  directory: (blockId) => directories.value?.get(blockId) ?? props.cwd ?? null,
  pictures: () => pictures.value,
})
const transcriptIndexById = computed(
  () =>
    new Map(visibleTranscriptBlocks.value.map((block, index) => [block.id, index]))
)

// A row's streaming and end follow its transcript block: a joined thinking
// step's last part.
function transcriptIndexAt(index: number): number {
  const block = renderBlocks.value[index]
  return block ? transcriptIndexById.value.get(followedBlockId(block)) ?? -1 : -1
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

// The next block's createdAt marks when a thinking block stopped; it is null
// while the block is still the last, that is, actively thinking. ThinkingBlock
// shows a frozen "thought for Xs" from it, which survives reload.
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
  useBlockVirtualizer(scrollContainer, renderBlocks, props.persistedScrollState, { keyOf })

onBeforeUnmount(() => {
  emit('saveScrollState', props.conversationId, getPersistedState())
})

// The runs the reader or a reveal opened, which stay open while the list shows
// this transcript.
const runs = useOpenRuns(() => renderBlocks.value, () => props.conversationId, keyOf)
// The block a search result opened. It is shown once the history holds it and
// the list has placed itself, and it is marked as it appears. The next request,
// or leaving the list, ends a mark that still waits.
let revealing: AbortController | null = null
/**
 * Brings the block `id`'s own row into view and marks it for a moment. Returns
 * false when the list does not hold the block.
 *
 * Every reveal of this list goes this way: a search result's, a notice's call's
 * and a receipt's block's alike. A block folded in a run opens the run first.
 * The block's own step row, never the run's, is the one placed and marked
 * (`crates/agent-session/src/reports.rs`).
 */
function revealAndMark(id: string): boolean {
  const scroller = scrollContainer.value
  const target = scroller ? runs.reveal(id) : null
  if (!scroller || !target)
    return false
  const rowSelector = `[data-block-id="${CSS.escape(target.row)}"]`
  const step = target.step === null ? null : `[data-step-id="${CSS.escape(target.step)}"]`
  // A run that opens draws its step's row in the next render. The list places
  // the row at once and the step as the row draws it, and the mark waits for
  // it.
  reveal(target.row, props.bottomOffset, step ?? undefined)
  revealing?.abort()
  revealing = new AbortController()
  highlightFound(scroller, step === null ? rowSelector : `${rowSelector} ${step}`, revealing.signal, { scroll: false })
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
// A report row's title brings the call that started its command into view. That
// call is the first one whose view names the command; a later one is only a
// look. The host finds a call the window does not hold.
const findCommand = useCommandRevealer()
provideCommandRevealer((commandId) => {
  const call = props.blocks.find((block) => block.type === 'tool_call' && storedShellView(block)?.commandId === commandId)
  return call
    ? () => {
        revealAndMark(call.id)
      }
    : findCommand(commandId)
})
// A receipt's name opens where it came from. When the window holds the block,
// the name opens it in this transcript; otherwise the host opens it elsewhere.
const openOrigin = useOriginRevealer()
provideOriginRevealer((origin) => {
  const { blockId } = origin
  const here = origin.node === (props.node ?? null) && blockId !== null && props.blocks.some((block) => block.id === blockId)
  return here
    ? () => {
        revealAndMark(blockId)
      }
    : openOrigin(origin)
})

const { height: viewportHeight } = useElementSize(scrollContainer)

useFollowSentMessages(() => renderBlocks.value, rowKey, scrollToBottom)

// Within a screen of an edge that is not the transcript's, the next page is
// read there, and the page before is read while the window shows a stale
// block. The host reads each page once.
watch(
  [scrollOffset, () => props.blocks, viewportHeight, () => props.atStart, () => props.atEnd, () => props.revealBlockId],
  () => {
    const el = scrollContainer.value
    // A block being brought into view places the list first. Until then, the
    // list's scroll position belongs to the window shown before.
    if (!el || paneStatus.value || props.revealBlockId) {
      return
    }
    // A stale block, held from before a reset that could not vouch for it,
    // is a hole the host reads again while it shows (`resetTranscript`).
    if ((!props.atStart && el.scrollTop < el.clientHeight) || props.blocks.some((block) => block.stale)) {
      emit('readBefore')
    }
    if (!props.atEnd && distanceFromBottom(el) < el.clientHeight) {
      emit('readAfter')
    }
  },
  { flush: 'post' },
)

// Composer or task-control growth covers the tail, and a reader at the bottom
// stays there. `bottomOffset` includes the permanent control row, even when its
// scroll button is hidden.
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

// The list's own height changes, as when a phone's on-screen keyboard opens
// over the page and the app shrinks above it (`phone-page.ts`). A reader at the
// end stays there, with the latest in view.
let listHeight: number | null = null
useResizeObserver(scrollContainer, ([entry]) => {
  const height = entry?.contentRect.height ?? null
  const changed = listHeight !== null && height !== listHeight
  listHeight = height
  if (changed && isAtBottom.value)
    scrollToBottom()
})

defineExpose({
  isAtBottom,
  scrollToBottom,
})
</script>

<template>
  <!--
    The transcript is read as one text: its rows are antialiased as its prose
    is, so they never draw heavier beside it.
  -->
  <div
    class="relative h-full antialiased"
    :style="{ '--transcript-clearance': `${props.bottomOffset + COMPOSER_CLEARANCE_PX}px` }"
  >
    <!--
      The scroll padding keeps a control the keyboard focuses out from under the
      dock, as the padding keeps the last block.
    -->
    <ScrollArea
      ref="scrollArea"
      class="h-full"
      :viewport-class="paneStatus ? '[overflow-anchor:none] [scroll-padding-bottom:var(--transcript-clearance)] flex flex-col' : '[overflow-anchor:none] [scroll-padding-bottom:var(--transcript-clearance)]'"
      @scroll="onScroll"
    >
      <SessionStatus
        v-if="paneStatus"
        :kind="paneStatus"
        :detail="paneStatus === 'failed' ? loadError : null"
        @retry="emit('retryLoad')"
      />
      <div
        v-else
        class="w-full px-[var(--session-gutter,0px)] pt-2"
        :style="{ paddingBottom: 'var(--transcript-clearance)' }"
      >
        <div ref="rowsRoot" class="relative w-full" :style="{ height: `${totalSize}px` }">
          <ReplyComments v-if="commenting" ref="comments" :root="rowsRoot" :commenting="commenting" :sent="sent" :asking="asking" />
          <div
            v-for="item in virtualItems"
            :key="String(item.key)"
            :data-index="item.index"
            :data-block-id="renderBlocks[item.index]!.id"
            :data-row-key="String(item.key)"
            :ref="(el) => measureElement(el as Element)"
            class="absolute inset-x-0 top-0 [--found-inset-x:max(0px,calc(var(--agent-pad-x,2rem)-0.5rem))]"
            :style="{ transform: `translateY(${item.start}px)` }"
          >
            <MessageEditRegion :muted="mutedIds.has(renderBlocks[item.index]!.id)">
              <AgentMessageVirtualBlock
                :block="renderBlocks[item.index]!"
                :open="runs.isOpen(keyOf(renderBlocks[item.index]!))"
                :is-thinking-streaming="isStreamingThinkingAt(item.index)"
                :is-text-streaming="isStreamingTextAt(item.index)"
                :show-assistant-footer="footerIds.has(renderBlocks[item.index]!.id)"
                :thinking-ended-at="thinkingEndedAt(item.index)"
                :fork="fork ? () => forkMessage(renderBlocks[item.index]!.id) : undefined"
                :fork-state="forkStates.get(renderBlocks[item.index]!.id)"
                :share="share ? answerShare(renderBlocks[item.index]!.id) : undefined"
                :hide-fork="hideFork"
                :failure="failures?.[renderBlocks[item.index]!.id]"
                :summary-tokens="summaryTokens.get(renderBlocks[item.index]!.id)"
                :entering="isEntering(String(item.key))"
                :editable="renderBlocks[item.index]!.id === editableUserId"
                :regenerate="renderBlocks[item.index]!.id === regenerateId ? regenerate : undefined"
                :versions="versionChoices.get(renderBlocks[item.index]!.id)"
                @update:open="(open) => runs.setOpen(keyOf(renderBlocks[item.index]!), open)"
                @recall-steer="(id) => emit('recallSteer', id)"
                @interrupt-pending-steer="(id) => emit('interruptPendingSteer', id)"
                @recall-queued="(id) => emit('recallQueued', id)"
                @send-queued="(id) => emit('sendQueued', id)"
                @edit-user="(id) => emit('editUser', id)"
                @edit-submission="(id) => emit('editSubmission', id)"
                @delete-submission="(id) => emit('deleteSubmission', id)"
                @retry-load="emit('retryLoad')"
              >
              </AgentMessageVirtualBlock>
            </MessageEditRegion>
          </div>
        </div>
      </div>
    </ScrollArea>
  </div>
</template>

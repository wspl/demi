<script setup lang="ts">
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import type { ProviderFailureFacts } from '@demicodes/protocol'
import { computed, useAttrs } from 'vue'
import { chromeEntrance } from '@demicodes/web-ui/ui/chrome-enter'
import { waitingAs, type MessageListBlock } from '../pending-steers'
import { rowFace } from '../row-key'
import RowRoll from '../../ui/RowRoll.vue'
import ErrorNotice from '../../ui/ErrorNotice.vue'
import type { PendingSubmissionState } from '../types'
import type { VersionChoice } from '../versions'
import UserBlock from './UserBlock.vue'
import { useMessageOrigin } from '../message-origins'
import ThinkingBlock from './ThinkingBlock.vue'
import AgentReceiptBlock from './AgentReceiptBlock.vue'
import CommandReportBlock from './CommandReportBlock.vue'
import AssistantTextBlock from './AssistantTextBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import PendingCallBlock from './PendingCallBlock.vue'
import WorkGroupBlock from './WorkGroupBlock.vue'
import StepsNotSharedBlock from './StepsNotSharedBlock.vue'
import ErrorBlock from './ErrorBlock.vue'
import DeliveryFailure from './DeliveryFailure.vue'
import AccountSwitchBlock from './AccountSwitchBlock.vue'
import StoppedBlock from './StoppedBlock.vue'
import CompactionBlock from './CompactionBlock.vue'
import QueueDivider from './QueueDivider.vue'
import PendingSubmission from '../PendingSubmission.vue'
import ActivitySlot from './ActivitySlot.vue'
import type { MessageForkState } from '../message-fork'
import type { AnswerShare } from '../sharing'
import { provideBlockScope } from '../whole-blocks'
import IndeterminateSpinner from '../../ui/IndeterminateSpinner.vue'
import { useTranscript } from '../edit-selection'
import { childBrief } from '../subagents'
import { blockMessageFiles } from '../working-directory'
import { bubbleName, receiptOrigin, receivedAsBubble, useOriginRevealer } from '../agent-receipts'
import { provideMessageFiles, useMessageFiles } from '../../markdown/message-files'

defineOptions({ inheritAttrs: false })

/** Who sent a message the user did not write, such as an automation's run. */
const originOf = useMessageOrigin()
const props = defineProps<{
  block: MessageListBlock
  isThinkingStreaming: boolean
  isTextStreaming?: boolean
  showAssistantFooter?: boolean
  thinkingEndedAt?: string | null
  editable?: boolean
  /** Regenerate in an answer's footer; absent where it is not offered. */
  regenerate?: () => void
  /**
   * The versions the row shows: an edited message's, or a regenerated answer's
   * in its footer.
   */
  versions?: VersionChoice
  fork?: () => Promise<void>
  forkState?: MessageForkState
  share?: AnswerShare
  /** No answer offers Fork, as on a share's page. */
  hideFork?: boolean
  /** What the provider read out of this error block's failure record. */
  failure?: ProviderFailureFacts
  /**
   * The block just arrived in a live transcript: a chrome row slides in from
   * the left as it fades in.
   */
  entering?: boolean
  /** The summary size a compaction divider tells. */
  summaryTokens?: number
}>()

/**
 * Whether a run's row, or the row of a step that runs under it, is open. The
 * list keeps it by the key it draws the row under (`useOpenRuns`), so a step
 * opened while it runs is still open as the run it starts. Other rows ignore
 * it.
 */
const runOpen = defineModel<boolean>('open', { default: false })

const emit = defineEmits<{
  recallSteer: [id: string]
  interruptPendingSteer: [id: string]
  recallQueued: [id: string]
  sendQueued: [id: string]
  editUser: [blockId: string]
  /**
   * Edit on the message `id`, whose delivery failed: it goes back into the
   * composer.
   */
  editSubmission: [id: string]
  /** Delete on the message `id`, whose delivery failed: it goes at once. */
  deleteSubmission: [id: string]
  /** Retry on a session-level failure: the session opens again. */
  retryLoad: []
}>()

const attrs = useAttrs()
const transcript = useTranscript()
// A child's messages name files from the directory it wrote them in, and
// show only the attachments it uploaded.
const files = useMessageFiles()
provideMessageFiles(() => blockMessageFiles(files(), transcript?.directory(props.block.id) ?? null, transcript?.node ?? null))
// A light block's row opens to read it whole.
provideBlockScope(() => ('createdAt' in props.block ? props.block : undefined))
/**
 * Rows built on FunctionalBlock: the 28px chrome face. Text streams in on its
 * own.
 */
const entersAsChrome = computed(() =>
  props.entering
  && (props.block.type === 'agent_message' || props.block.type === 'wakeup' || props.block.type === 'thinking' || props.block.type === 'tool_call' || props.block.type === 'pending_call' || props.block.type === 'work_group' || props.block.type === 'abort' || props.block.type === 'activity_slot'),
)

const revealOrigin = useOriginRevealer()
/**
 * Another agent's message this agent shows as a bubble with its words, as its
 * brief (`receivedAsBubble`), and the sender named above it, but for the main
 * agent; null for a message that keeps its receipt row.
 */
const bubble = computed(() => {
  const block = props.block
  if (block.type !== 'agent_message' || !receivedAsBubble(block)) {
    return null
  }
  const message = block.message
  const name = bubbleName(message)
  const origin = receiptOrigin(message)
  return {
    content: [{ type: 'text' as const, text: message.content }],
    brief: message.content,
    sender: name === null ? null : { name, go: origin ? revealOrigin(origin) : undefined },
  }
})

/**
 * Recall and Steer on the message being sent: the same requests as on
 * the steer or the queued message it will be, by the id it keeps, which the
 * host carries out once the session holds it.
 */
function recallSubmission(submission: PendingSubmissionState): void {
  if (submission.way === 'steer') {
    emit('recallSteer', submission.id)
  } else {
    emit('recallQueued', submission.id)
  }
}

function sendSubmissionNow(submission: PendingSubmissionState): void {
  if (submission.way === 'steer') {
    emit('interruptPendingSteer', submission.id)
  } else {
    emit('sendQueued', submission.id)
  }
}
</script>

<template>
  <!--
    A row that takes another's place rolls its face over the one it replaces
    (`rowFace`).
  -->
  <RowRoll :face-key="rowFace(block)">
    <PendingSubmission
      v-if="block.type === 'pending_submission'"
      v-bind="{ ...attrs, ...block.submission }"
      :pending="waitingAs(block)"
      @edit="emit('editSubmission', block.submission.id)"
      @delete="emit('deleteSubmission', block.submission.id)"
      @recall="recallSubmission(block.submission)"
      @send-now="sendSubmissionNow(block.submission)"
    />
    <UserBlock
      v-else-if="block.type === 'user'"
      v-bind="attrs"
      :content="block.content"
      :comments="block.comments"
      :origin="originOf(block.automation)"
      :brief="block.comments?.length ? null : childBrief(transcript?.node ?? null, block.content)"
      :editable="editable"
      :versions="versions"
      @edit="emit('editUser', block.id)"
    />
    <UserBlock
      v-else-if="bubble"
      v-bind="attrs"
      :content="bubble.content"
      :brief="bubble.brief"
      :sender="bubble.sender"
      :editable="false"
    />
    <UserBlock
      v-else-if="block.type === 'steer'"
      v-bind="attrs"
      :content="block.content"
      :comments="block.comments"
      :editable="false"
    />
    <UserBlock
      v-else-if="block.type === 'pending_steer'"
      v-bind="attrs"
      :content="block.content"
      :comments="block.comments"
      :pending="waitingAs(block)"
      @recall="emit('recallSteer', block.pendingSteerId)"
      @send-now="emit('interruptPendingSteer', block.pendingSteerId)"
    />
    <UserBlock
      v-else-if="block.type === 'queued_message'"
      v-bind="attrs"
      :content="block.content"
      :comments="block.comments"
      :origin="originOf(block.automation)"
      :pending="waitingAs(block)"
      @recall="emit('recallQueued', block.queueId)"
      @send-now="emit('sendQueued', block.queueId)"
    />
    <QueueDivider
      v-else-if="block.type === 'queue_divider'"
      v-bind="attrs"
      :count="block.count"
    />
    <StepsNotSharedBlock
      v-else-if="block.type === 'steps_not_shared'"
      v-bind="attrs"
      :count="block.count"
    />
    <!--
      The edge of a window that does not reach the transcript's own, while its
      next page comes.
    -->
    <div
      v-else-if="block.type === 'history_edge'"
      v-bind="attrs"
      class="flex h-9 items-center justify-center text-fg-faint"
      role="status"
      :aria-label="shared('common.loading')"
    >
      <IndeterminateSpinner :size="14" />
    </div>
    <div
      v-else
      v-bind="{ ...attrs, ...chromeEntrance(entersAsChrome) }"
    >
      <AgentReceiptBlock
        v-if="block.type === 'agent_message'"
        :message="block.message"
      />
      <CommandReportBlock
        v-else-if="block.type === 'wakeup'"
        :reports="block.reports"
      />
      <ThinkingBlock
        v-else-if="block.type === 'thinking'"
        v-model:open="runOpen"
        :thinking="block.text"
        :is-streaming="isThinkingStreaming"
        :created-at="block.createdAt"
        :ended-at="thinkingEndedAt"
      />
      <AssistantTextBlock
        v-else-if="block.type === 'text'"
        :block-id="block.id"
        :content="block.text"
        :code-files="block.codeFiles"
        :is-streaming="isTextStreaming"
        :show-footer="showAssistantFooter"
        :created-at="block.createdAt"
        :fork="block.forkable ? fork : undefined"
        :fork-state="forkState"
        :share="share"
        :share-wait="!block.forkable"
        :hide-fork="hideFork"
        :regenerate="regenerate"
        :versions="versions"
      />
      <div
        v-else-if="block.type === 'tool_call'"
        class="overflow-hidden px-[var(--agent-pad-x,2rem)]"
      >
        <ToolCallBlock
          v-model:open="runOpen"
          :block="block"
        />
      </div>
      <div
        v-else-if="block.type === 'pending_call'"
        class="overflow-hidden px-[var(--agent-pad-x,2rem)]"
      >
        <PendingCallBlock :step="block" />
      </div>
      <div
        v-else-if="block.type === 'work_group'"
        class="overflow-hidden px-[var(--agent-pad-x,2rem)]"
      >
        <WorkGroupBlock v-model:open="runOpen" :group="block" />
      </div>
      <div
        v-else-if="block.type === 'error'"
        class="px-[var(--agent-pad-x,2rem)] py-1.5"
      >
        <ErrorBlock
          :message="block.message"
          :code="block.code"
          :diagnostics="block.diagnostics"
          :provider-id="block.model.providerId"
          :retry-at="failure?.retryAt ?? null"
          :signed-out="failure?.signedOut ?? null"
          :not-exposed="failure?.notExposed ?? null"
          :device-offline="failure?.deviceOffline ?? null"
        />
      </div>
      <div
        v-else-if="block.type === 'abort'"
        class="px-[var(--agent-pad-x,2rem)]"
      >
        <StoppedBlock />
      </div>
      <div
        v-else-if="block.type === 'account_switch'"
        class="px-[var(--agent-pad-x,2rem)]"
      >
        <AccountSwitchBlock :block="block" />
      </div>
      <CompactionBlock
        v-else-if="block.type === 'compaction_marker' || block.type === 'compaction_boundary'"
        :summary-tokens="summaryTokens"
      />
      <CompactionBlock
        v-else-if="block.type === 'compaction_progress'"
        running
      />
      <ActivitySlot
        v-else-if="block.type === 'activity_slot'"
        :kind="block.slot.kind"
        :since="block.slot.since"
      />
      <div
        v-else-if="block.type === 'delivery_failure'"
        class="px-[var(--agent-pad-x,2rem)]"
        data-delivery-failure
      >
        <DeliveryFailure :submission="block.submission" />
      </div>
      <div
        v-else-if="block.type === 'session_failure'"
        class="px-[var(--agent-pad-x,2rem)] py-1.5"
      >
        <ErrorNotice
          :label="block.failure.label"
          :request-id="block.failure.requestId"
          :actions="block.failure.retry ? [{ label: shared('common.retry'), run: () => emit('retryLoad') }] : []"
        />
      </div>
    </div>
  </RowRoll>
</template>

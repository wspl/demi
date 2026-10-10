<script setup lang="ts">
import type { ProviderFailureFacts } from '@demicodes/protocol'
import { computed, useAttrs } from 'vue'
import { chromeEntrance } from '@demicodes/web-ui/ui/chrome-enter'
import type { MessageListBlock } from '../pending-steers'
import UserBlock from './UserBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import AgentReceiptBlock from './AgentReceiptBlock.vue'
import CommandReportBlock from './CommandReportBlock.vue'
import AssistantTextBlock from './AssistantTextBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import PendingCallBlock from './PendingCallBlock.vue'
import WorkGroupBlock from './WorkGroupBlock.vue'
import ErrorBlock from './ErrorBlock.vue'
import StoppedBlock from './StoppedBlock.vue'
import CompactionBlock from './CompactionBlock.vue'
import QueueDivider from './QueueDivider.vue'
import PendingSubmission from '../PendingSubmission.vue'
import type { MessageForkState } from '../message-fork'
import { provideBlockScope } from '../whole-blocks'
import IndeterminateSpinner from '../../ui/IndeterminateSpinner.vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  block: MessageListBlock
  isThinkingStreaming: boolean
  isTextStreaming?: boolean
  showAssistantFooter?: boolean
  thinkingEndedAt?: string | null
  editable?: boolean
  /** Regenerate in an answer's footer; absent where it is not offered. */
  regenerate?: () => void
  fork?: () => Promise<void>
  forkState?: MessageForkState
  /** What the provider read out of this error block's failure record. */
  failure?: ProviderFailureFacts
  /** The block just arrived in a live transcript: a chrome row slides in from the left as it fades in. */
  entering?: boolean
  /** The summary size a compaction divider tells. */
  summaryTokens?: number
}>()

const emit = defineEmits<{
  deletePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  deleteQueued: [id: string]
  sendQueued: [id: string]
  editUser: [blockId: string]
  retrySubmission: []
}>()

const attrs = useAttrs()
// A light block's row opens to read it whole (`web-api.md` § Light form).
provideBlockScope(() => ('createdAt' in props.block ? props.block : undefined))
/** Rows built on FunctionalBlock: the 28px chrome face. Text streams in on its own. */
const entersAsChrome = computed(() =>
  props.entering
  && (props.block.type === 'agent_message' || props.block.type === 'wakeup' || props.block.type === 'thinking' || props.block.type === 'tool_call' || props.block.type === 'pending_call' || props.block.type === 'work_group' || props.block.type === 'abort'),
)
</script>

<template>
  <PendingSubmission
    v-if="block.type === 'pending_submission'"
    v-bind="{ ...attrs, ...block.submission }"
    @retry="emit('retrySubmission')"
  />
  <UserBlock
    v-else-if="block.type === 'user'"
    v-bind="attrs"
    :content="block.content"
    :editable="editable"
    @edit="emit('editUser', block.id)"
  />
  <UserBlock
    v-else-if="block.type === 'steer'"
    v-bind="attrs"
    :content="block.content"
    :editable="false"
  />
  <UserBlock
    v-else-if="block.type === 'pending_steer'"
    v-bind="attrs"
    :content="block.content"
    pending="steer"
    @remove="emit('deletePendingSteer', block.pendingSteerId)"
    @send-now="emit('interruptPendingSteer', block.pendingSteerId)"
  />
  <UserBlock
    v-else-if="block.type === 'queued_message'"
    v-bind="attrs"
    :content="block.content"
    pending="queued"
    @remove="emit('deleteQueued', block.queueId)"
    @send-now="emit('sendQueued', block.queueId)"
  />
  <QueueDivider
    v-else-if="block.type === 'queue_divider'"
    v-bind="attrs"
    :count="block.count"
  />
  <!-- The edge of a window that does not reach the transcript's own, while its next page comes. -->
  <div
    v-else-if="block.type === 'history_edge'"
    v-bind="attrs"
    class="flex h-9 items-center justify-center text-fg-faint"
    role="status"
    aria-label="Loading"
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
      :thinking="block.text"
      :is-streaming="isThinkingStreaming"
      :created-at="block.createdAt"
      :ended-at="thinkingEndedAt"
    />
    <AssistantTextBlock
      v-else-if="block.type === 'text'"
      :content="block.text"
      :is-streaming="isTextStreaming"
      :show-footer="showAssistantFooter"
      :created-at="block.createdAt"
      :fork="block.forkable ? fork : undefined"
      :fork-state="forkState"
      :regenerate="regenerate"
    />
    <div
      v-else-if="block.type === 'tool_call'"
      class="overflow-hidden px-[var(--agent-pad-x,2rem)]"
    >
      <ToolCallBlock
        :block="block"
      />
    </div>
    <div
      v-else-if="block.type === 'pending_call'"
      class="overflow-hidden px-[var(--agent-pad-x,2rem)]"
    >
      <PendingCallBlock :call="block.call" />
    </div>
    <div
      v-else-if="block.type === 'work_group'"
      class="overflow-hidden px-[var(--agent-pad-x,2rem)]"
    >
      <WorkGroupBlock :group="block" />
    </div>
    <div
      v-else-if="block.type === 'error'"
      class="px-[var(--agent-pad-x,2rem)] py-1.5"
    >
      <ErrorBlock
        :message="block.message"
        :code="block.code"
        :diagnostics="block.diagnostics"
        :retry-at="failure?.retryAt ?? null"
      />
    </div>
    <div
      v-else-if="block.type === 'abort'"
      class="px-[var(--agent-pad-x,2rem)]"
    >
      <StoppedBlock />
    </div>
    <CompactionBlock
      v-else-if="block.type === 'compaction_marker' || block.type === 'compaction_boundary'"
      :summary-tokens="summaryTokens"
    />
    <CompactionBlock
      v-else-if="block.type === 'compaction_progress'"
      running
    />
  </div>
</template>

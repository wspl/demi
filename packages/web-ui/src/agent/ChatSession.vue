<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Archive, Play } from '@lucide/vue'
import type { TranscriptVersion } from '@demicodes/agent/client'
import { beginMessageEdit, lastEditableUserMessageId, type MessageEditState } from './message-editing'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import TerminalChip from '@demicodes/web-ui/agent/TerminalChip.vue'
import TerminalPanel from '@demicodes/web-ui/agent/TerminalPanel.vue'
import { useSessionPanels } from './useSessionPanels'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import type { ChatSessionState, PendingSubmissionState } from './types'
import type { MessageForkHandler } from './message-fork'

import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { t } from '../infra/i18n'
import { sessionFailureNotice } from './session-status'
import { getVisibleBlocks } from './visible-blocks'
import type { PersistedScrollState } from '../composables/useBlockVirtualizer'

const props = defineProps<{
  conversation: ChatSessionState
  hasProvider: boolean
  pendingSubmission?: PendingSubmissionState | null
  editVersion?: TranscriptVersion | null
  messageEdit?: MessageEditState | null
  fork?: MessageForkHandler
}>()
const emit = defineEmits<{
  archive: []
  retry: []
  retryLoad: []
  retrySubmission: []
  abortSubagents: []
  abortSubagent: [id: string]
  abortTerminal: [id: string]
  removeQueued: [id: string]
  sendQueued: [id: string]
  removePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  'update:messageEdit': [state: MessageEditState | null]
  saveScroll: [id: string, state: PersistedScrollState | null]
}>()
const surface = ref<{ dockHeight: number }>()
// Recovery (Retry on the tail error record, Resume after an abort) needs a
// provider and a conversation that is neither archived nor being edited.
const canRecover = computed(
  () =>
    props.hasProvider &&
    !props.messageEdit &&
    !props.conversation.archived &&
    props.conversation.load !== 'failed',
)
// A session-level failure is told once, in the transcript flow: never beside
// the status pane that already replaced the transcript, never under an error
// record that already carries Retry.
const failureNotice = computed(() => {
  const visible = getVisibleBlocks(props.conversation.blocks)
  return sessionFailureNotice(
    props.conversation.load,
    props.conversation.lastError,
    visible.length > 0,
    visible.at(-1)?.type === 'error',
  )
})
const canEdit = computed(() => !!props.editVersion && !props.messageEdit
  && !props.pendingSubmission
  && !props.conversation.archived
  && !props.conversation.subagents.some((agent) => agent.phase === 'running'))

function editUser(id: string): void {
  const block = props.conversation.blocks.find((item) => item.id === id)
  if (!block || !props.editVersion || !canEdit.value
    || id !== lastEditableUserMessageId(props.conversation.blocks)) {
    return
  }
  emit('update:messageEdit', beginMessageEdit(block, props.editVersion))
}
const list = ref<{
  isAtBottom: boolean
  scrollToBottom: () => void
}>()
const { activeSubagentId, activeTerminalId, toggleAgents, toggleTerminals, close } =
  useSessionPanels(
    () => props.conversation.subagents,
    () => props.conversation.terminals,
  )
watch(() => props.conversation.id, close)
</script>

<template>
  <section
    class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-surface"
  >
    <header
      class="grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2 gap-y-1 px-3 py-2"
    >
      <div class="col-span-2 flex min-w-0 items-center gap-1 sm:col-span-1">
        <h1
          class="min-w-0 select-none truncate text-chrome font-normal text-fg"
          :title="conversation.title"
        >
          {{ conversation.title }}
        </h1>
        <Tooltip
          content="Archive"
          class="shrink-0"
        >
          <IconButton
            :icon="Archive"
            variant="ghost"
            aria-label="Archive"
            :disabled="conversation.phase !== 'idle' || conversation.archived"
            @click="emit('archive')"
          />
        </Tooltip>
      </div>
      <div
        class="col-span-2 row-start-2 min-w-0 sm:col-span-1 sm:col-start-2 sm:row-start-1"
      >
        <slot name="workspace" />
      </div>
    </header>
    <div class="relative min-h-0 flex-1 overflow-hidden">
      <SessionSurface ref="surface">
        <div class="flex h-full min-h-0 flex-col">
          <AgentMessageList
            ref="list"
            class="min-h-0 flex-1"
            :key="conversation.id"
            :conversation-id="conversation.id"
            :blocks="conversation.blocks"
            :queue="conversation.queue"
            :pending-steers="conversation.pendingSteers"
            :phase="conversation.phase"
            :load="conversation.load"
            :pending-action="conversation.pendingAction"
            :load-error="conversation.lastError"
            :failure="failureNotice"
            :retry="canRecover ? () => emit('retry') : undefined"
            :pending-submission="pendingSubmission"
            :read-only="!canEdit"
            :fork="fork"
            :edit-target-id="messageEdit?.request.targetBlockId"
            @retry-submission="emit('retrySubmission')"
            :bottom-offset="surface?.dockHeight ?? 0"
            :persisted-scroll-state="conversation.scroll ?? undefined"
            @save-scroll-state="(id, state) => emit('saveScroll', id, state ?? null)"
            @delete-queued="(id) => emit('removeQueued', id)"
            @send-queued="(id) => emit('sendQueued', id)"
            @delete-pending-steer="(id) => emit('removePendingSteer', id)"
            @interrupt-pending-steer="(id) => emit('interruptPendingSteer', id)"
            @edit-user="editUser"
            @retry-load="emit('retryLoad')"
          />
        </div>
        <template #dock>
          <SessionDock
            :show-scroll-to-bottom="!!list && !list.isAtBottom"
            @scroll-to-bottom="list?.scrollToBottom()"
          >
            <template #chips>
              <!-- An error record carries its own Retry in the transcript; only an abort resumes from here. -->
              <SessionDockChip
                v-if="canRecover && conversation.status === 'aborted'"
                @click="emit('retry')"
              >
                <Play :size="ICON_PX.in28" />
                {{ t('agent.dock.resume') }}
              </SessionDockChip>
              <TerminalChip
                :terminals="conversation.terminals"
                :open="activeTerminalId !== null"
                @open="toggleTerminals"
              />
              <AgentsChip
                :agents="conversation.subagents"
                :open="activeSubagentId !== null"
                @open="toggleAgents"
              />
            </template>
            <slot
              v-if="
                conversation.archived ||
                conversation.load === 'ready' ||
                conversation.load === 'reconnecting'
              "
              name="composer"
            />
          </SessionDock>
        </template>
        <template #overDock>
          <SubagentPanel
            v-model:active-id="activeSubagentId"
            :agents="conversation.subagents"
            @abort="emit('abortSubagents')"
            @abort-agent="(id) => emit('abortSubagent', id)"
          />
          <TerminalPanel
            v-model:active-id="activeTerminalId"
            :terminals="conversation.terminals"
            @abort="(id) => emit('abortTerminal', id)"
          />
        </template>
      </SessionSurface>
    </div>
  </section>
</template>

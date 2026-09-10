<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Archive, Play, RotateCcw } from '@lucide/vue'
import type { TranscriptVersion } from '@demicodes/agent/client'
import MessageEditDialog from './MessageEditDialog.vue'
import { beginMessageEdit, type MessageEditState } from './message-editing'
import { appOverlayStore } from '../overlay/appOverlay'
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
import SessionNoticeBar from '@demicodes/web-ui/agent/SessionNoticeBar.vue'
import type { PendingSubmissionState } from './types'

import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { ConversationState } from './types'
import type { ConversationStatus } from './conversation-status'
import type { SubagentRecord } from './subagents'
import type { TerminalRecord } from './terminals'
import type { PersistedScrollState } from '../composables/useBlockVirtualizer'

export interface ChatSessionState
  extends Pick<
    ConversationState,
    | 'id'
    | 'title'
    | 'blocks'
    | 'queue'
    | 'pendingSteers'
    | 'phase'
    | 'load'
    | 'lastError'
  > {
  archived: boolean
  status: ConversationStatus
  scroll: PersistedScrollState | null
  subagents: SubagentRecord[]
  terminals: TerminalRecord[]
}
const props = defineProps<{
  conversation: ChatSessionState
  hasProvider: boolean
  pendingSubmission?: PendingSubmissionState | null
  editVersion?: TranscriptVersion | null
  messageEdit?: MessageEditState | null
}>()
const emit = defineEmits<{
  archive: []
  retry: []
  retryLoad: []
  retrySubmission: []
  abortSubagents: []
  removeQueued: [id: string]
  sendQueued: [id: string]
  removePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  'update:messageEdit': [state: MessageEditState | null]
  submitEdit: []
  saveScroll: [id: string, state: PersistedScrollState | null]
}>()
const surface = ref<{ dockHeight: number }>()
const editorOpen = ref(false)
const canEdit = computed(() => !!props.editVersion && !props.messageEdit
  && !props.conversation.archived
  && !props.conversation.subagents.some((agent) => agent.phase === 'running'))

function editUser(id: string): void {
  const block = props.conversation.blocks.find((item) => item.id === id)
  if (!block || !props.editVersion || !canEdit.value) {
    return
  }
  emit('update:messageEdit', beginMessageEdit(block, props.editVersion))
  editorOpen.value = true
}

function cancelEdit(): void {
  emit('update:messageEdit', null)
  editorOpen.value = false
}

watch(() => props.conversation.id, () => { editorOpen.value = false })
watch(() => props.messageEdit, (state) => {
  if (!state) {
    editorOpen.value = false
  }
})
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
          content="Archive conversation"
          class="shrink-0"
        >
          <IconButton
            :icon="Archive"
            variant="ghost"
            aria-label="Archive conversation"
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
            :pending-submission="pendingSubmission"
            :read-only="!canEdit"
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
              <SessionDockChip v-if="messageEdit" @click="editorOpen = true">
                {{ messageEdit.phase === 'sending' ? 'Saving edited message…' : 'Review edited message' }}
              </SessionDockChip>
              <SessionNoticeBar
                v-if="conversation.lastError"
                :label="conversation.lastError"
              />
              <SessionDockChip
                v-if="
                  !conversation.archived &&
                  hasProvider &&
                  (conversation.status === 'error' ||
                    conversation.status === 'aborted')
                "
                @click="emit('retry')"
              >
                <component
                  :is="conversation.status === 'error' ? RotateCcw : Play"
                  :size="ICON_PX.in28"
                />
                {{ conversation.status === 'error' ? 'Retry' : 'Resume' }}
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
          />
          <TerminalPanel
            v-model:active-id="activeTerminalId"
            :terminals="conversation.terminals"
          />
        </template>
      </SessionSurface>
    </div>
  </section>
  <MessageEditDialog
    v-if="messageEdit"
    :is-open="editorOpen"
    :overlay-store="appOverlayStore"
    :state="messageEdit"
    @close="editorOpen = false"
    @cancel="cancelEdit"
    @update="emit('update:messageEdit', $event)"
    @submit="emit('submitEdit')"
  />
</template>

<script setup lang="ts">
import { provideEditReads, provideEditSelection, type EditSelectionHandler } from './edit-selection'
import type { ReadCallChange } from '../files/changes'
import { providePageOpening, type PageOpeningHandler } from './page-opening'
import { provideMessageFiles } from '../markdown/message-files'
import type { ConversationFiles } from '../markdown/types'
import { computed, ref, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { PanelRight, Play, Radar, TextCursorInput } from '@lucide/vue'
import type { TranscriptVersion } from '@demicodes/protocol'
import {
  beginMessageEdit,
  offeredEditId,
  provideEditLastMessage,
  unchangedEditRequest,
  type MessageEditRequest,
  type MessageEditState,
} from './message-editing'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import TerminalChip from '@demicodes/web-ui/agent/TerminalChip.vue'
import TerminalPanel from '@demicodes/web-ui/agent/TerminalPanel.vue'
import { useSessionPanels } from './useSessionPanels'
import { provideLiveCalls } from './live-calls'
import { commandTitles, provideCommandReferences } from './command-references'
import { callTerminal, dockTerminals } from './terminals'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import type { ChatSessionState, PendingSubmissionState } from './types'
import type { MessageForkHandler } from './message-fork'

import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import TitleInput from '@demicodes/web-ui/ui/TitleInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { provideLabelRoom } from '../ui/label-room'
import { sessionFailureNotice, turnRecovery } from './session-status'
import { getVisibleBlocks } from './visible-blocks'
import type { PersistedScrollState } from '../composables/useBlockVirtualizer'
import PermissionCard from '../permissions/PermissionCard.vue'
import type { PermissionDecision, PermissionRequestView } from '../permissions/types'

const props = withDefaults(defineProps<{
  conversation: ChatSessionState
  hasProvider: boolean
  pendingSubmission?: PendingSubmissionState | null
  editVersion?: TranscriptVersion | null
  messageEdit?: MessageEditState | null
  selectEdit?: EditSelectionHandler
  /** Reads a retained edit's two sides, for a work group's file counts. */
  readEdit?: ReadCallChange
  /** Opens a page the agent presented; without it, its card offers no Open. */
  openPage?: PageOpeningHandler
  fork?: MessageForkHandler
  /**
   * Detect Title in the Rename menu: `available` while a message is newer
   * than the last generated title, `running` while the model writes one, which
   * disables it. Absent, the title is current and there is nothing to detect.
   * The title shows no progress: when the model's title arrives, it changes.
   */
  retitle?: 'available' | 'running' | null
  /** Whether the app frame's work panel is open; absent when the host has none. */
  asideOpen?: boolean
  /** The conversation's Host files its messages name; absent, their paths stay text. */
  files?: ConversationFiles
  /** The conversation's undecided permission requests, oldest first, which the card above the composer shows. */
  permissionRequests?: readonly PermissionRequestView[]
  /** A decision on a permission request is on its way. */
  decidingPermission?: boolean
  /** A message to bring into view and mark for a moment, as a search result opened at it asks. */
  revealBlockId?: string | null
  /**
   * The host's connection banner says the backend is away
   * (`web-application.md` § A page of another build): the transcript adds no
   * Connecting row of its own while the conversation's socket waits for it.
   */
  backendAway?: boolean
}>(), {
  // Vue reads an absent boolean prop as false, which would offer Open panel
  // where there is no panel; absent stays undefined.
  asideOpen: undefined,
})
const emit = defineEmits<{
  openAside: []
  rename: [title: string]
  retitle: []
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
  /**
   * Regenerate: the edit of the last message with its content unchanged,
   * which the host sends at once through `regenerateMessage`.
   */
  regenerate: [request: MessageEditRequest]
  saveScroll: [id: string, state: PersistedScrollState | null]
  decidePermission: [id: string, decision: PermissionDecision]
  /** The message `revealBlockId` names is in view. */
  revealed: []
}>()
provideEditSelection(() => props.selectEdit)
provideEditReads(() => props.readEdit)
providePageOpening(() => props.openPage)
// A running call shows its command's output under it; once the call
// returned, a command that still runs is the dock's (`runtime.md`
// § Rendering boundary).
provideLiveCalls((toolUseId) => callTerminal(props.conversation.terminals, undefined, toolUseId))
// A look or a wait may name any agent's command; a list names its own
// transcript's with a jump, and these by their titles alone.
const conversationCommands = computed(() =>
  commandTitles([
    ...props.conversation.blocks,
    ...props.conversation.subagents.flatMap((agent) => agent.blocks),
  ]),
)
provideCommandReferences((commandId) => conversationCommands.value(commandId))
const terminals = computed(() =>
  dockTerminals(props.conversation.terminals, (subagentId) =>
    subagentId === undefined
      ? props.conversation.blocks
      : props.conversation.subagents.find((agent) => agent.id === subagentId)?.blocks ?? [],
  ),
)
// Relative paths resolve against the directory the conversation works in.
provideMessageFiles(() => props.files && { ...props.files, cwd: props.conversation.cwd })
const surface = ref<{ dockHeight: number }>()
// The title is the last thing in the header to be cut short, up to its
// greatest width: the controls beside it give up their labels before a title
// narrower than that is cut, and keep them while a longer one is cut to it.
const TITLE_MAX_PX = 260
/** The Rename button beside the title, and the gap before it. */
const TITLE_ACTION_PX = 28 + 4
const renaming = ref(false)

/** A double-click on the title names it in place, as the menu's Rename does. */
function beginRename(): void {
  if (!props.conversation.archived) {
    renaming.value = true
  }
}
const titleCell = ref<HTMLElement | null>(null)
const title = ref<HTMLElement | null>(null)
/**
 * The title cell's width, unknown until it is first laid out; its room is
 * unknown until then, and every label shows. Taken for zero wide, the cell
 * would hide the labels beside it and bring them back within its first
 * measurement, resizing itself inside its own ResizeObserver callback.
 */
const titleCellWidth = ref<number>()
useResizeObserver(titleCell, ([entry]) => {
  titleCellWidth.value = entry?.contentRect.width
})
const titleRoom = ref<number>()
watch(
  [titleCellWidth, () => props.conversation.title, title, renaming],
  () => {
    if (titleCellWidth.value === undefined) {
      return
    }
    // The input a rename swaps in is as wide as a title gets, and leaves no button beside it.
    const need = title.value
      ? Math.min(title.value.scrollWidth, TITLE_MAX_PX) + TITLE_ACTION_PX
      : TITLE_MAX_PX
    titleRoom.value = titleCellWidth.value - need
  },
  { flush: 'post' },
)
provideLabelRoom(() => titleRoom.value)
// Recovery of an unfinished turn needs a provider and a conversation that is
// neither archived nor being edited. A message the user sent, still on its
// way or waiting for the backend, is the other way forward.
const canRecover = computed(
  () =>
    props.hasProvider &&
    !props.messageEdit &&
    !props.pendingSubmission &&
    !props.conversation.archived &&
    props.conversation.load !== 'failed',
)
// A session-level failure is told once, in the transcript flow: never beside
// the status pane that already replaced the transcript, never under an error
// record that already says it.
const failureNotice = computed(() => {
  const visible = getVisibleBlocks(props.conversation.blocks)
  return sessionFailureNotice(
    props.conversation.load,
    props.conversation.lastError,
    visible.length > 0,
    visible.at(-1)?.type === 'error',
  )
})
// One control in the dock: Resume after an error, Continue after the user's Stop.
const recovery = computed(() =>
  canRecover.value
    ? turnRecovery(props.conversation.phase, getVisibleBlocks(props.conversation.blocks))
    : null,
)
const canEdit = computed(() => !!props.editVersion && !props.messageEdit
  && !props.pendingSubmission
  && !props.conversation.archived
  && !props.conversation.subagents.some((agent) => agent.phase === 'running'))

/** The message the page offers to edit, and to regenerate the answer of; null for none. */
const offeredId = computed(() => canEdit.value ? offeredEditId(props.conversation) : null)

/** The offered message's block, with the version an edit of it is taken at. */
function offered(id: string) {
  const block = props.conversation.blocks.find((item) => item.id === id)
  return block && props.editVersion && id === offeredId.value
    ? { block, version: props.editVersion }
    : null
}

function editUser(id: string): void {
  const target = offered(id)
  if (target) {
    emit('update:messageEdit', beginMessageEdit(target.block, target.version))
  }
}

function regenerate(id: string): void {
  const target = offered(id)
  if (target) {
    emit('regenerate', unchangedEditRequest(target.block, target.version))
  }
}

// Up Arrow in an empty composer opens the editor on the offered message.
provideEditLastMessage(() => {
  const id = offeredId.value
  return id === null ? undefined : () => editUser(id)
})
const list = ref<{
  isAtBottom: boolean
  scrollToBottom: () => void
}>()
const { activeSubagentId, activeTerminalId, toggleAgents, toggleTerminals, close } =
  useSessionPanels(
    () => props.conversation.subagents,
    () => terminals.value,
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
      <div ref="titleCell" class="col-span-2 flex min-w-0 items-center gap-1 sm:col-span-1">
        <TitleInput
          v-if="renaming"
          :style="{ width: `${TITLE_MAX_PX}px` }"
          :title="conversation.title"
          @submit="renaming = false; emit('rename', $event)"
          @cancel="renaming = false"
        />
        <template v-else>
          <h1
            ref="title"
            class="min-w-0 select-none truncate text-chrome font-normal text-fg"
            :style="{ maxWidth: `${TITLE_MAX_PX}px` }"
            :title="conversation.title"
            @dblclick="beginRename"
          >
            {{ conversation.title }}
          </h1>
          <!-- Two ways to name the conversation behind one button. -->
          <Dropdown
            :overlay-store="appOverlayStore"
            :disabled="conversation.archived"
          >
            <template #trigger="{ isOpen }">
              <Tooltip content="Rename" class="shrink-0">
                <IconButton
                  :icon="TextCursorInput"
                  variant="ghost"
                  aria-label="Rename"
                  :pressed="isOpen"
                  :disabled="conversation.archived"
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  :icon="Radar"
                  label="Detect Title"
                  :disabled="retitle !== 'available'"
                  :disabled-reason="retitle === 'running'
                    ? 'A title is being detected'
                    : 'No new message since the last detected title'"
                  @select="close(); emit('retitle')"
                />
                <MenuItem
                  :icon="TextCursorInput"
                  label="Rename"
                  @select="close(); renaming = true"
                />
              </Menu>
            </template>
          </Dropdown>
        </template>
      </div>
      <div
        class="col-span-2 row-start-2 flex min-w-0 items-center gap-1 sm:col-span-1 sm:col-start-2 sm:row-start-1"
      >
        <div class="min-w-0 flex-1">
          <slot name="workspace" />
        </div>
        <!-- Only while the panel is closed: open, its own fold control closes it. -->
        <Tooltip
          v-if="asideOpen === false"
          content="Open panel"
          class="shrink-0"
        >
          <IconButton
            :icon="PanelRight"
            variant="ghost"
            aria-label="Open panel"
            @click="emit('openAside')"
          />
        </Tooltip>
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
            :pending-calls="conversation.pendingCalls"
            :queue="conversation.queue"
            :pending-steers="conversation.pendingSteers"
            :phase="conversation.phase"
            :load="conversation.load"
            :backend-away="backendAway"
            :pending-action="conversation.pendingAction"
            :failures="conversation.failures"
            :load-error="conversation.lastError"
            :failure="failureNotice"
            :pending-submission="pendingSubmission"
            :read-only="!canEdit"
            :fork="fork"
            :edit-target-id="messageEdit?.request.targetBlockId"
            @retry-submission="emit('retrySubmission')"
            :bottom-offset="surface?.dockHeight ?? 0"
            :persisted-scroll-state="conversation.scroll ?? undefined"
            :reveal-block-id="revealBlockId"
            @revealed="emit('revealed')"
            @save-scroll-state="(id, state) => emit('saveScroll', id, state ?? null)"
            @delete-queued="(id) => emit('removeQueued', id)"
            @send-queued="(id) => emit('sendQueued', id)"
            @delete-pending-steer="(id) => emit('removePendingSteer', id)"
            @interrupt-pending-steer="(id) => emit('interruptPendingSteer', id)"
            @edit-user="editUser"
            @regenerate="regenerate"
            @retry-load="emit('retryLoad')"
          />
        </div>
        <template #dock>
          <SessionDock
            :show-scroll-to-bottom="!!list && !list.isAtBottom"
            @scroll-to-bottom="list?.scrollToBottom()"
          >
            <template v-if="permissionRequests?.length" #above>
              <PermissionCard
                :requests="permissionRequests"
                :deciding="decidingPermission"
                @decide="(id, decision) => emit('decidePermission', id, decision)"
              />
            </template>
            <template #chips>
              <!-- The transcript says what happened; the one recovery control is here, over the input. -->
              <SessionDockChip v-if="recovery" @click="emit('retry')">
                <Play :size="ICON_PX.in28" />
                {{ recovery === 'resume' ? 'Resume' : 'Continue' }}
              </SessionDockChip>
              <TerminalChip
                :terminals="terminals"
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
            :terminals="conversation.terminals"
            @abort="emit('abortSubagents')"
            @abort-agent="(id) => emit('abortSubagent', id)"
          />
          <TerminalPanel
            v-model:active-id="activeTerminalId"
            :terminals="terminals"
            @abort="(id) => emit('abortTerminal', id)"
          />
        </template>
      </SessionSurface>
    </div>
  </section>
</template>

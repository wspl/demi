<script setup lang="ts">
import { t } from './locales/text'
import { t as shared } from '../locales/text'
import type { ShownError } from '../infra/errors'
import { provideEditSelection, provideFilePills, type EditSelectionHandler } from './edit-selection'
import { provideResumeAtRetry } from './resume-at-retry'
import { provideCredentialKinds, type CredentialKinds } from './credential-kinds'
import { provideMessageFiles } from '../markdown/message-files'
import type { ConversationFiles } from '../markdown/types'
import { computed, nextTick, ref, toRaw, watch } from 'vue'
import { useResizeObserver } from '@vueuse/core'
import { ArrowRightLeft, ChevronDown, Ellipsis, PanelLeft, PanelRight, Play, Settings2, Share, TextCursorInput } from '@lucide/vue'
import type { InstructionEntry, TranscriptVersion } from '@demicodes/protocol'
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
import { DOCK_WINDOW_SHARE } from '@demicodes/web-ui/agent/dock-window'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import TerminalChip from '@demicodes/web-ui/agent/TerminalChip.vue'
import TerminalPanel from '@demicodes/web-ui/agent/TerminalPanel.vue'
import { useSessionPanels } from './useSessionPanels'
import { provideLiveCalls } from './live-calls'
import { useCallTerminals, useDockTerminals } from './terminals'
import { provideCommandRevealer } from './command-reports'
import { provideOriginRevealer } from './agent-receipts'
import { agentLinkWindow, findAgent, provideMessageAgents } from './agent-links'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import type { ChatSessionState, PendingSubmissionState } from './types'
import type { HostMenuHost } from '../hosts/types'
import type { MessageForkHandler } from './message-fork'
import { useShare, type ExistingShare, type ShareService } from './sharing'
import ShareDialog from './ShareDialog.vue'
import type { UsageLimits } from './context-usage'

import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { isTextCut } from '@demicodes/web-ui/ui/truncation'
import TitleInput from '@demicodes/web-ui/ui/TitleInput.vue'
import { conversationTitle } from './conversation-title'
import Sheet from '../ui/Sheet.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import RenameDialog from '../ui/RenameDialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import MenuEntries from '../ui/MenuEntries.vue'
import { selectionEntries } from '../sidebar/selection-menu'
import ConversationDeleteDialog from '../sidebar/ConversationDeleteDialog.vue'
import { copyId } from './copy-id'
import StatusDot from '../ui/StatusDot.vue'
import AutomationsChip from '../automations/AutomationsChip.vue'
import type { AutomationLink } from '../automations/types'
import { usePhone } from '../ui/phone'
import { useSidebarDrawer } from '../sidebar/sidebar-drawer'
import { DEVICE_STATE_LABEL, DEVICE_STATE_TONE } from '../devices/state'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { provideLabelRoom } from '../ui/label-room'
import { continueWithOffer, endsWithErrorRecord, signedOutOffer, sessionFailureNotice, sessionShown, transcriptShown, turnRecovery } from './session-status'
import { getVisibleBlocks } from './visible-blocks'
import { latestWindow } from '@demicodes/conversation-client'
import { latestBlocks, shownWindow, windowEdges, type TranscriptReveal } from './history'
import { provideBlockReader, type BlockReader } from './whole-blocks'
import type { PersistedScrollState } from '../composables/useBlockVirtualizer'
import RequestCard from '../permissions/RequestCard.vue'
import type { CredentialAnswer, PermissionDecision, RequestView } from '../permissions/types'

const props = withDefaults(defineProps<{
  conversation: ChatSessionState
  hasProvider: boolean
  /** The messages sent and not yet confirmed, in the order they were sent. */
  pendingSubmissions?: readonly PendingSubmissionState[]
  /**
   * The waiting messages whose Recall is on its way, by id: their rows are
   * gone from the press, and come back only when the removal is refused
   * (`turns.md` § Steer or queue).
   */
  removing?: readonly string[]
  editVersion?: TranscriptVersion | null
  messageEdit?: MessageEditState | null
  selectEdit?: EditSelectionHandler
  fork?: MessageForkHandler
  /**
   * Shares the conversation, and an answer by its footer's Share, as the
   * share dialog asks (`sharing.md`); absent, the conversation has nothing to
   * share yet, as one not saved, and neither control is offered.
   */
  share?: ShareService
  /** The conversation's shares, which the dialog opens on instead of making another. */
  shares?: readonly ExistingShare[]
  /** Whether the app frame's work panel is open; absent when the host has none. */
  asideOpen?: boolean
  /**
   * The conversation as the sidebar lists it: on a phone its title sheet
   * offers the sidebar row's menu for it. Absent, as for a conversation not
   * saved yet or archived, the sheet offers only where it runs.
   */
  listed?: { pinned: boolean }
  /**
   * The automations that wake the conversation (`automations.md` § Where the
   * user sees them): a chip above the composer lists them.
   */
  automations?: readonly AutomationLink[]
  /** The conversation's Host files its messages name; absent, their paths stay text. */
  files?: ConversationFiles
  /**
   * The conversation's current primary Host, live over its state updates:
   * the dock's Resume continues an unfinished turn there, so it waits while
   * that Host is an offline device. Absent, Resume waits for nothing.
   */
  primaryHost?: HostMenuHost | null
  /** The conversation's undecided requests of any kind, oldest first, which the card above the composer shows. */
  permissionRequests?: readonly RequestView[]
  /** A decision on a request is on its way. */
  decidingPermission?: boolean
  /** What the backend refused the last credential answer for, shown under the card's form. */
  requestError?: ShownError | null
  /**
   * A block to bring into view and mark for a moment, as a search result
   * opened at it or a report's title asks: in the list, or in the subagent
   * panel, which then shows that subagent.
   */
  reveal?: TranscriptReveal | null
  /**
   * Finds the call that started a command a report names when the window
   * shown does not hold it: the host reads where it lies and sets `reveal`
   * (`web-api-conversations.md` § Subagents and commands). Absent, such a title is no link.
   */
  revealCommand?: (commandId: string) => void
  /** Reads a block whole for a row the page holds light (`web-api-conversations.md` § Light form); absent, no row is light. */
  readBlock?: BlockReader
  /**
   * The usage limits of the account a provider entry runs on, which a
   * child's context card in the Agents window shows; absent, it shows none.
   */
  usageLimits?: (providerId: string, account: string | null) => UsageLimits | null
  /**
   * What each provider entry holds, which a failure's record names, as *The
   * provider refused the API key*; absent, records read their code's words.
   */
  credentialKinds?: CredentialKinds
  /**
   * The host's connection banner says the backend is away
   * (`web-application.md` § A page of another build): the transcript adds no
   * Connecting row of its own while the conversation's socket waits for it.
   */
  backendAway?: boolean
  /**
   * The user's preference to resume a turn by itself at the time its failure
   * names, which such a failure's record offers; absent, it offers none.
   */
  resumeAtRetry?: boolean
  /**
   * Whether calls and folded runs show the files they changed as pills; the
   * product shows none for a conversation outside a project
   * (`edit-tracking.md` § What the conversation shows). Absent, they show.
   */
  filePills?: boolean
}>(), {
  // Vue reads an absent boolean prop as false, which would offer Open panel
  // where there is no panel; absent stays undefined.
  asideOpen: undefined,
  resumeAtRetry: undefined,
  filePills: true,
})
/** The share of the session area's height the dock's windows keep; the host keeps it (`dock-window.ts`). */
const dockWindowShare = defineModel<number>('dockWindowShare', { default: DOCK_WINDOW_SHARE.default })

const sharing = useShare(() => props.share, () => props.shares ?? [])
const shareConversation = { kind: 'conversation' } as const

const emit = defineEmits<{
  /** Open an automation that wakes the conversation, from its mark. */
  openAutomation: [id: string]
  /** The dock windows' share settled: a drag ended, a key or a double-click applied. */
  dockWindowResizeEnd: [share: number]
  /** A failure's record turned the preference to resume at its time on or off. */
  'update:resumeAtRetry': [on: boolean]
  openAside: []
  rename: [title: string]
  /** The row menu's commands, from the phone's title sheet; Delete once the user confirmed it. */
  pin: [pinned: boolean]
  archive: []
  delete: []
  retry: []
  /**
   * The failed turn's usage limit offered another account of its entry,
   * without rotation: the host makes it active, then resumes
   * (`providers.md` § Account rotation).
   */
  continueWith: [providerId: string, credentialId: string]
  /** Open Settings › Providers, where an account the vendor signed out is signed in again. */
  openProviders: []
  retryLoad: []
  /** Retry on the message `id`, whose delivery failed. */
  retrySubmission: [id: string]
  abortSubagents: []
  abortSubagent: [id: string]
  abortTerminal: [id: string]
  recallQueued: [id: string]
  sendQueued: [id: string]
  recallSteer: [id: string]
  interruptPendingSteer: [id: string]
  'update:messageEdit': [state: MessageEditState | null]
  /**
   * Regenerate: the edit of the last message with its content unchanged,
   * which the host sends at once through `regenerateMessage`.
   */
  regenerate: [request: MessageEditRequest]
  /** Show the `to`th version at the place of the block `blockId` (`message-editing.md` § Versions). */
  switchVersion: [blockId: string, to: number]
  saveScroll: [id: string, state: PersistedScrollState | null]
  decidePermission: [id: string, decision: PermissionDecision, credential?: CredentialAnswer]
  /** The block `reveal` names is in view. */
  revealed: []
  /**
   * Bring this block into view and mark it, as a receipt's name asks
   * (`subagents.md` § Product rendering): the host reads the window around
   * it and sets `reveal`.
   */
  reveal: [at: TranscriptReveal]
  /** The reader nears the start of the window shown, which does not reach the transcript's (`web-application.md` § Transcript windows). */
  readBefore: []
  /** The reader nears the end of the window shown, which does not reach the transcript's. */
  readAfter: []
  /** Show the latest window, as the scroll-to-bottom button asks while another is shown. */
  showLatest: []
  /** Read a page of a subagent's transcript, for its panel. */
  readSubagent: [id: string, at: 'latest' | 'before']
  /** Compact in a child's context card, in the Agents window: that child's context. */
  compactSubagent: [id: string]
  /** A child's context card asks for the usage limits of the account it runs on, `account` of its entry `providerId`, again. */
  refreshLimits: [providerId: string, account: string | null, automatic: boolean]
  /** An instruction in a child's context card opens where it is written. */
  openInstruction: [entry: InstructionEntry]
}>()
provideEditSelection(() => props.selectEdit)
provideFilePills(() => props.filePills)
provideResumeAtRetry(() => props.resumeAtRetry === undefined
  ? undefined
  : { on: props.resumeAtRetry, set: (on) => emit('update:resumeAtRetry', on) })
provideBlockReader(() => props.readBlock)
provideCredentialKinds(() => props.credentialKinds)
// A running call shows its command's output under it; once the call
// returned, a command that still runs is the dock's (`rendering.md`
// § Rendering boundary).
const commandOf = useCallTerminals(() => props.conversation.terminals)
provideLiveCalls((toolUseId) => commandOf(undefined, toolUseId))
// A report's title whose call the window shown does not hold asks the host where it lies.
provideCommandRevealer((commandId) => {
  const find = props.revealCommand
  return find && (() => {
    find(commandId)
  })
})
const terminals = useDockTerminals(() => props.conversation)
/** The window of the transcript the list shows, and where it stands. */
const shown = computed(() => shownWindow(props.conversation.history, props.conversation.shownAt))
const shownEdges = computed(() => windowEdges(props.conversation.history, shown.value))
const shownLatest = computed(() => shown.value === latestWindow(props.conversation.history) || shownEdges.value.atEnd)
/**
 * The blocks at the transcript's end, which a turn, its recovery and the
 * message offered to edit read, as plain data: a block that changes is
 * replaced, never changed in place, so the window's new list is the change
 * to follow, and reading every block through its proxy at each streamed
 * piece would track every field of a long transcript.
 */
const latest = computed(() => toRaw(latestBlocks(props.conversation.history)).map(toRaw))
/** The blocks at the transcript's end the reader sees. */
const latestVisible = computed(() => getVisibleBlocks(latest.value))
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

// A phone shows one top bar (`SidebarLayout`): the sidebar's button, the
// title, which opens the title sheet with what the desktop's header shows
// beside the title, and the panel's button.
const phone = usePhone()
const drawer = useSidebarDrawer()
const titleSheetOpen = ref(false)
/** Where the title sheet shows the conversation's place, while it is open. */
const workspaceTarget = ref<HTMLElement | null>(null)
/** The primary Host's state on the title, while it is not ready to run. */
const hostNotReady = computed(() => {
  const host = props.primaryHost
  return host && host.state !== 'online' ? host : null
})
/** On a phone, a rename asks in a dialog, as a phone's own apps do; the bar has no room for the field. */
const renameAsked = computed(() => renaming.value && phone.value)

/** Closes the title sheet, then does what its item chose. */
function fromTitleSheet(then: () => void): void {
  titleSheetOpen.value = false
  then()
}
/** Delete… from the title sheet asks first, as the sidebar's does. */
const deleteAsked = ref(false)

/** The conversation's commands in the title sheet: the sidebar row's, but Open, since it is the one shown. */
function titleSheetEntries(listed: { pinned: boolean }) {
  return selectionEntries([{ id: props.conversation.id, pinned: listed.pinned }], {
    open: () => undefined,
    rename: () => fromTitleSheet(() => (renaming.value = true)),
    copyId: () => fromTitleSheet(() => void copyId(props.conversation.id)),
    pin: (_ids, pinned) => fromTitleSheet(() => emit('pin', pinned)),
    archive: () => fromTitleSheet(() => emit('archive')),
    delete: () => fromTitleSheet(() => (deleteAsked.value = true)),
  }, true)
}

/** The Rename button, or a double-click on the title, names it in place. */
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
/** The title the session shows, the placeholder's in the reader's language. */
const shownTitle = computed(() => conversationTitle(props.conversation.title))
const titleCellWidth = ref<number>()
useResizeObserver(titleCell, ([entry]) => {
  titleCellWidth.value = entry?.contentRect.width
})
const titleRoom = ref<number>()
watch(
  [titleCellWidth, shownTitle, title, renaming],
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
// neither archived nor being edited, whose session has told its turn. A
// message the user sent, still on its way or waiting for the backend, is the
// other way forward.
const canRecover = computed(
  () =>
    props.hasProvider &&
    !props.messageEdit &&
    !props.pendingSubmissions?.length &&
    !props.conversation.archived &&
    sessionShown(props.conversation.load),
)
// A session-level failure is told once, in the transcript flow: never beside
// the status pane that already replaced the transcript, never under an error
// record that already says it, also when receipts arrived behind it.
const failureNotice = computed(() => {
  const visible = latestVisible.value
  return sessionFailureNotice(
    props.conversation.load,
    props.conversation.lastError,
    visible.length > 0,
    endsWithErrorRecord(visible),
  )
})
// One control in the dock: Resume after an error, Continue after the user's Stop.
const recovery = computed(() =>
  canRecover.value
    ? turnRecovery(props.conversation.phase, latestVisible.value)
    : null,
)
function recover(): void {
  emit('retry')
}
/**
 * The account a usage limit of an entry without rotation offers to go on
 * with, the one with the most room as the failure's facts name it, beside
 * Resume; none for any other failure.
 */
const continueWith = computed(() =>
  canRecover.value
    ? continueWithOffer(props.conversation.phase, latestVisible.value, props.conversation.failures)
    : null,
)
/** Whether the turn ended because the vendor signed its account out, which the dock answers with Open Providers. */
const signedOut = computed(() =>
  canRecover.value && signedOutOffer(props.conversation.phase, latestVisible.value, props.conversation.failures),
)
const canEdit = computed(() => !!props.editVersion && !props.messageEdit
  && !props.pendingSubmissions?.length
  && !props.conversation.archived
  && !props.conversation.subagents.some((agent) => agent.phase === 'running'))

/** The message the page offers to edit, and to regenerate the answer of; null for none. */
const offeredId = computed(() => canEdit.value
  ? offeredEditId({ ...props.conversation, blocks: latest.value })
  : null)

/** The offered message's block, with the version an edit of it is taken at. */
function offered(id: string) {
  const block = latest.value.find((item) => item.id === id)
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
/** The button's way to the end: to the latest window when another is shown, then to its end. */
function scrollToBottom(): void {
  if (!shownLatest.value) {
    emit('showLatest')
    void nextTick(() => list.value?.scrollToBottom())
    return
  }
  list.value?.scrollToBottom()
}
/**
 * The children the Agents window and `agent:` links show: those started on
 * the current path; a child of another version of a message stays stored
 * and comes back with its version (`message-editing.md` § Versions).
 */
const shownAgents = computed(() => props.conversation.subagents.filter((agent) => agent.onPath))
const { agentsOpen, activeSubagentId, activeTerminalId, toggleAgents, toggleTerminals, close } =
  useSessionPanels(
    () => shownAgents.value,
    () => terminals.value,
    () => (props.conversation.load === 'loading' ? null : props.conversation.id),
  )
// A receipt's name opens the Agents window on its sender's tab, a finished
// sender as Completed picks it, and the host brings the block it names into
// view there; the root's block, in the transcript.
provideOriginRevealer(({ node, blockId }) => {
  if (node === null && blockId === null) {
    return undefined
  }
  return () => {
    if (node !== null) {
      activeSubagentId.value = node
    }
    if (blockId !== null) {
      emit('reveal', { node, blockId })
    }
  }
})
// A message's link to an agent opens the Agents window on its tab, and
// leaves it as it is when it shows that tab already; agent 0's brings this
// transcript into view (`subagents.md` § Linking an agent).
provideMessageAgents(() => ({
  find: (number) => findAgent(shownAgents.value, number),
  open: (number) => {
    const next = agentLinkWindow(
      { open: agentsOpen.value, activeId: activeSubagentId.value },
      shownAgents.value,
      number,
    )
    if (!next) {
      return
    }
    if (next.open) {
      activeSubagentId.value = next.activeId
    } else {
      close()
    }
  },
}))
// A block in a subagent's transcript shows in the panel, on that subagent's tab.
watch(
  () => props.reveal?.node,
  (node) => {
    if (node) {
      activeSubagentId.value = node
    }
  },
)
</script>

<template>
  <section
    class="flex min-h-0 flex-1 flex-col overflow-hidden bg-surface md:rounded-tl-xl"
  >
    <header
      v-if="phone"
      class="flex h-[calc(2.75rem+var(--safe-top))] shrink-0 select-none items-center gap-1 px-1.5 pt-[var(--safe-top)]"
    >
      <IconButton
        v-if="drawer"
        :icon="PanelLeft"
        variant="ghost"
        :aria-label="t('session.openSidebar')"
        @click="drawer.open()"
      />
      <!-- The title is centered between the bar's two buttons, whichever of them shows. -->
      <span v-else class="size-hit shrink-0" />
      <button
        type="button"
        class="flex h-full min-w-0 flex-1 items-center justify-center gap-1.5 px-1 text-fg"
        :aria-label="t('session.options', { title: shownTitle })"
        :aria-expanded="titleSheetOpen"
        @click="titleSheetOpen = true"
      >
        <StatusDot
          v-if="hostNotReady"
          :tone="DEVICE_STATE_TONE[hostNotReady.state]"
          :label="t('session.hostState', { name: hostNotReady.name, state: DEVICE_STATE_LABEL[hostNotReady.state].toLowerCase() })"
        />
        <h1 class="min-w-0 truncate text-[15px] font-medium">{{ shownTitle }}</h1>
        <ChevronDown :size="ICON_PX.in20" class="shrink-0 text-fg-subtle" />
      </button>
      <IconButton
        v-if="asideOpen !== undefined"
        :icon="PanelRight"
        variant="ghost"
        :aria-label="t('session.openPanel')"
        @click="emit('openAside')"
      />
      <span v-else class="size-hit shrink-0" />
      <!-- A menu sheet: the title it acts on stays in the bar above it, so the sheet repeats no heading. -->
      <Sheet :open="titleSheetOpen" :label="shownTitle" :overlay-store="appOverlayStore" @close="titleSheetOpen = false">
        <Menu v-if="listed || share">
          <template v-if="share">
            <MenuItem
              :icon="Share"
              :label="t('session.shareConversation')"
              @select="fromTitleSheet(() => void sharing.start(shareConversation))"
            />
            <MenuDivider v-if="listed" />
          </template>
          <MenuEntries
            v-if="listed"
            :groups="titleSheetEntries(listed)"
          />
        </Menu>
        <!-- Where the conversation runs, as the desktop's header shows it beside the title. -->
        <div v-if="$slots.workspace" class="px-4 pb-3 pt-1">
          <p class="pb-1 text-detail text-fg-subtle">{{ t('session.runsOn') }}</p>
          <div ref="workspaceTarget" />
        </div>
      </Sheet>
      <!-- The place stays mounted while the sheet is closed, as the desktop's header keeps it: the
           composer's offline card moves the conversation through it. The sheet shows it while open. -->
      <div class="hidden">
        <Teleport :to="workspaceTarget ?? 'body'" :disabled="workspaceTarget === null">
          <slot name="workspace" />
        </Teleport>
      </div>
      <RenameDialog
        :is-open="renameAsked"
        :overlay-store="appOverlayStore"
        :title="t('session.renameTitle')"
        :label="t('session.titleLabel')"
        :name="shownTitle"
        :max-length="null"
        @close="renaming = false"
        @rename="renaming = false; emit('rename', $event)"
      />
      <ConversationDeleteDialog
        :is-open="deleteAsked"
        :overlay-store="appOverlayStore"
        :titles="[shownTitle]"
        @close="deleteAsked = false"
        @delete="deleteAsked = false; emit('delete')"
      />
    </header>
    <header
      v-else
      class="grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2 gap-y-1 px-3 py-2"
    >
      <div ref="titleCell" class="col-span-2 flex min-w-0 items-center gap-1 sm:col-span-1">
        <TitleInput
          v-if="renaming"
          :style="{ width: `${TITLE_MAX_PX}px` }"
          :title="shownTitle"
          @submit="renaming = false; emit('rename', $event)"
          @cancel="renaming = false"
        />
        <template v-else>
          <h1
            ref="title"
            class="min-w-0 select-none truncate text-chrome font-normal text-fg"
            :style="{ maxWidth: `${TITLE_MAX_PX}px` }"
            @dblclick="beginRename"
          >
            {{ shownTitle }}
          </h1>
          <!-- A cut title shows whole in its tip, as a sidebar row's does. -->
          <Tooltip :trigger="title" :content="shownTitle" :show-if="isTextCut" placement="bottom-start" />
          <Tooltip :content="shared('common.rename')" class="shrink-0">
            <IconButton
              :icon="TextCursorInput"
              variant="ghost"
              :aria-label="shared('common.rename')"
              :disabled="conversation.archived"
              @click="beginRename"
            />
          </Tooltip>
        </template>
      </div>
      <div
        class="col-span-2 row-start-2 flex min-w-0 items-center gap-1 sm:col-span-1 sm:col-start-2 sm:row-start-1"
      >
        <div class="min-w-0 flex-1">
          <slot name="workspace" />
        </div>
        <Dropdown v-if="share" :overlay-store="appOverlayStore">
          <template #trigger="{ isOpen }">
            <Tooltip :content="t('panel.more')" class="shrink-0">
              <IconButton
                :icon="Ellipsis"
                variant="ghost"
                :aria-label="t('panel.more')"
                :pressed="isOpen"
              />
            </Tooltip>
          </template>
          <template #content="{ close }">
            <Menu @click="close">
              <MenuItem
                :icon="Share"
                :label="t('session.shareConversation')"
                @select="sharing.start(shareConversation)"
              />
            </Menu>
          </template>
        </Dropdown>
        <!-- Only while the panel is closed: open, its own fold control closes it. -->
        <Tooltip
          v-if="asideOpen === false"
          :content="t('session.openPanel')"
          class="shrink-0"
        >
          <IconButton
            :icon="PanelRight"
            variant="ghost"
            :aria-label="t('session.openPanel')"
            @click="emit('openAside')"
          />
        </Tooltip>
      </div>
    </header>
    <!-- A bar about where the conversation belongs, under the header: an automation's run's. -->
    <slot name="bar" />
    <div class="relative min-h-0 flex-1 overflow-hidden">
      <SessionSurface
        ref="surface"
        v-model:dock-window-share="dockWindowShare"
        @dock-window-resize-end="emit('dockWindowResizeEnd', $event)"
      >
        <div class="flex h-full min-h-0 flex-col">
          <AgentMessageList
            ref="list"
            class="min-h-0 flex-1"
            :key="conversation.id"
            :conversation-id="conversation.id"
            :blocks="shown.blocks"
            :at-start="shownEdges.atStart"
            :at-end="shownEdges.atEnd"
            :latest="shownLatest"
            :summaries="conversation.summaries"
            :pending-calls="conversation.pendingCalls"
            :waiting-since="conversation.waitingSince"
            :queue="conversation.queue"
            :pending-steers="conversation.pendingSteers"
            :phase="conversation.phase"
            :load="conversation.load"
            :backend-away="backendAway"
            :pending-action="conversation.pendingAction"
            :failures="conversation.failures"
            :load-error="conversation.lastError"
            :failure="failureNotice"
            :pending-submissions="pendingSubmissions"
            :removing="removing"
            :read-only="!canEdit"
            :fork="fork"
            :share="share ? (blockId) => sharing.start({ kind: 'answer', blockId }) : undefined"
            :edit-target-id="messageEdit?.request.targetBlockId"
            @retry-submission="(id) => emit('retrySubmission', id)"
            :bottom-offset="surface?.dockHeight ?? 0"
            :persisted-scroll-state="conversation.scroll ?? undefined"
            :reveal-block-id="reveal?.node === null ? reveal.blockId : null"
            @revealed="emit('revealed')"
            @save-scroll-state="(id, state) => emit('saveScroll', id, state ?? null)"
            @recall-queued="(id) => emit('recallQueued', id)"
            @send-queued="(id) => emit('sendQueued', id)"
            @recall-steer="(id) => emit('recallSteer', id)"
            @interrupt-pending-steer="(id) => emit('interruptPendingSteer', id)"
            @edit-user="editUser"
            @regenerate="regenerate"
            @switch-version="(id, to) => emit('switchVersion', id, to)"
            @retry-load="emit('retryLoad')"
            @read-before="emit('readBefore')"
            @read-after="emit('readAfter')"
          />
        </div>
        <template #dock>
          <SessionDock
            :show-scroll-to-bottom="!shownLatest || (!!list && !list.isAtBottom)"
            @scroll-to-bottom="scrollToBottom"
          >
            <template v-if="permissionRequests?.length" #above>
              <RequestCard
                :requests="permissionRequests"
                :deciding="decidingPermission"
                :error="requestError"
                @decide="(id, decision, credential) => emit('decidePermission', id, decision, credential)"
              />
            </template>
            <template #chips>
              <!-- The transcript says what happened; the one recovery control is here, over the input. -->
              <SessionDockChip v-if="recovery" @click="recover">
                <Play :size="ICON_PX.in28" />
                {{ recovery === 'resume' ? t('dock.resume') : t('dock.continue') }}
              </SessionDockChip>
              <SessionDockChip
                v-if="continueWith"
                @click="emit('continueWith', continueWith.providerId, continueWith.credentialId)"
              >
                <ArrowRightLeft :size="ICON_PX.in28" />
                <span class="min-w-0 truncate">{{ t('dock.continueWith', { label: continueWith.label }) }}</span>
              </SessionDockChip>
              <SessionDockChip v-if="signedOut" data-open-providers @click="emit('openProviders')">
                <Settings2 :size="ICON_PX.in28" />
                {{ t('dock.openProviders') }}
              </SessionDockChip>
              <TerminalChip
                :terminals="terminals"
                :open="activeTerminalId !== null"
                @open="toggleTerminals"
              />
              <AgentsChip
                :agents="shownAgents"
                :open="agentsOpen"
                @open="toggleAgents"
              />
              <AutomationsChip
                v-if="automations?.length"
                :automations="automations"
                @open="(id) => emit('openAutomation', id)"
              />
            </template>
            <!-- With the transcript: a message sent while the session opens goes once it has. -->
            <slot
              v-if="conversation.archived || transcriptShown(conversation.load)"
              name="composer"
            />
          </SessionDock>
        </template>
        <template #overDock>
          <SubagentPanel
            v-model:open="agentsOpen"
            v-model:active-id="activeSubagentId"
            :agents="shownAgents"
            :terminals="conversation.terminals"
            @abort="emit('abortSubagents')"
            @abort-agent="(id) => emit('abortSubagent', id)"
            :reveal="reveal?.node ? reveal : null"
            @read="(id, at) => emit('readSubagent', id, at)"
            @revealed="emit('revealed')"
            :usage-limits="usageLimits"
            @compact-agent="(id) => emit('compactSubagent', id)"
            @refresh-limits="(providerId, account, automatic) => emit('refreshLimits', providerId, account, automatic)"
            @open-instruction="(entry) => emit('openInstruction', entry)"
          />
          <TerminalPanel
            v-model:active-id="activeTerminalId"
            :terminals="terminals"
            @abort="(id) => emit('abortTerminal', id)"
          />
        </template>
      </SessionSurface>
    </div>
    <ShareDialog
      v-if="sharing.shown.value"
      :is-open="sharing.isOpen.value"
      :overlay-store="appOverlayStore"
      :kind="sharing.shown.value.target.kind"
      :phase="sharing.shown.value"
      :copied="sharing.copied.value"
      :on-update="sharing.update"
      :on-stop="sharing.stop"
      @copy="sharing.copy"
      @retry="sharing.retry"
      @close="sharing.close"
    />
  </section>
</template>

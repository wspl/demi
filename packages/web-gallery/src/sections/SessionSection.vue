<script setup lang="ts">
import { GALLERY_MARKS, shownAmong } from '../fixtures/device-marks'
import { correction, requestFailure } from '../fixtures/request-failures'
import type { ShownError } from '@demicodes/web-ui/infra/errors'
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { Layers, Play } from '@lucide/vue'
import { countOf } from '@demicodes/utils'
import ThinkingBlock from '@demicodes/web-ui/agent/blocks/ThinkingBlock.vue'
import AgentReceiptBlock from '@demicodes/web-ui/agent/blocks/AgentReceiptBlock.vue'
import { callBeingWrittenBlocks, commandLookBlocks, editedFile, viewedMediaBlocks, movedReceiptMessage, organizeReceiptMessages, permissionReceiptMessages } from '../fixtures/blocks'
import {
  SUBAGENT,
  subagentBlocks,
  subagentParentBlocks,
  movedRequestBlocks,
  cleanupRequestBlocks,
  signInRequestBlocks,
  standaloneRequest,
  uncopiedRequestBlocks,
} from '../fixtures/request-changes'
import { useGalleryTranscripts } from '../fixtures/transcripts'
import GalleryTranscript from '../components/GalleryTranscript.vue'
import GalleryStepPlaces from '../components/GalleryStepPlaces.vue'
import { stepPlaces } from '../fixtures/step-places'
import GalleryEditSelection from '../components/GalleryEditSelection.vue'
import GalleryFilePills from '../components/GalleryFilePills.vue'
import RequestCard from '@demicodes/web-ui/permissions/RequestCard.vue'
import {
  afterDecision,
  type PermissionDecision,
  type RequestView,
} from '@demicodes/web-ui/permissions/types'
import {
  allowanceRequest,
  FIELDS_DIFFER,
  deployTokenRequest,
  googleKeyRequest,
  INVALID_ANSWER,
  hostRequest,
  pairingRequest,
  queuedRequests,
  removedAllowanceRequest,
  renewalRequest,
  rootRequest,
  signInRequest,
  subagentRequest,
} from '../fixtures/permissions'
import { readGalleryEdit } from '../fixtures/blobs'
import ErrorBlock from '@demicodes/web-ui/agent/blocks/ErrorBlock.vue'
import ToolShellBlock from '@demicodes/web-ui/agent/blocks/ToolShellBlock.vue'
import ToolCallBlock from '@demicodes/web-ui/agent/blocks/ToolCallBlock.vue'
import { parseToolInput } from '@demicodes/web-ui/agent/block-helpers'
import ActivitySlot from '@demicodes/web-ui/agent/blocks/ActivitySlot.vue'
import type { ActivityKind } from '@demicodes/web-ui/agent/activity-slot'
import { provideEditSelection } from '@demicodes/web-ui/agent/edit-selection'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import GalleryWorkPanel from '../components/GalleryWorkPanel.vue'
import { galleryFiles, useGalleryWork, type GalleryWork } from '../fixtures/work-panel'
import {
  changePath,
  firstChangeData,
  goBack as changeBack,
  goForward as changeForward,
  holdShownChange,
  placeBranch,
  showBranch,
  showChange,
  showCommit,
  showEdit,
  showItem,
  showRequestEdit,
  type ChangeData,
} from '@demicodes/plugin-changes/data'
import { LINKED_COMMIT, sharedHistory } from '../fixtures/history'
import { COMMITS_HEIGHT } from '@demicodes/web-ui/files/file-view'
import type { CommitHistorySource } from '@demicodes/web-ui/files/history'
import { selectedTab } from '@demicodes/web-ui/agent/panel-tabs'
import type { PanelTabKind } from '@demicodes/web-ui/agent/panel-kinds/kind'
import { BrowserTabsError, browserTabDataSchema, startingLabel, type StartingPhase } from '@demicodes/plugin-browser/live/tabs'
import { SERVER_FAILED } from '../fixtures/plugins'
import type { ChangeMode, ChangeSetSource, ChangeSources } from '@demicodes/web-ui/files/changes'
import { storedRequests, type RequestChangeSource, type RequestEditRef, type RequestEditSelection } from '@demicodes/web-ui/files/request-changes'
import ChangeView from '@demicodes/web-ui/files/ChangeView.vue'
import FileView from '@demicodes/web-ui/files/FileView.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { createGalleryChangeSet, createGalleryWorkspace, gallerySides, QUOTED_CODE, SLOT_ROOT } from '../fixtures/workspace'
import { galleryBrowser, type GalleryBrowser } from '../fixtures/live-browser'
import { galleryConversationFiles, namedFilesMarkdown } from '../fixtures/message-files'
import GalleryMessageFiles from '../components/GalleryMessageFiles.vue'
import GalleryAgentLinks from '../components/GalleryAgentLinks.vue'
import { attachmentMarkdown, audioAttachmentMarkdown, edgedImagesMarkdown, fiveMediaMarkdown, listAttachmentMarkdown, loneImageMarkdown, twoMediaMarkdown } from '../fixtures/attachments'
import { productWould } from '../product-would'
import { useGalleryShares } from '../fixtures/sharing'
import { useAgentContext } from '../agent-context'
import { showRecalled } from '@demicodes/web-ui/agent/recalled-toast'
import { latestBlocks, type TranscriptReveal } from '@demicodes/web-ui/agent/history'
import { sidebarEntries } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../generated/pages'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SearchDialog from '@demicodes/web-ui/search/SearchDialog.vue'
import type { SearchRow } from '@demicodes/web-ui/search/search'
import { fixtureSearch, searchableMessages } from '../fixtures/search'
import { ASIDE_SHARE, SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
import { demoAccount, demoConversations, demoProjects } from '../sidebar/sidebar-data'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import DockWindowArea from '@demicodes/web-ui/agent/DockWindowArea.vue'
import { dockWindowShare } from '../dock-window-share'
import UserBlock from '@demicodes/web-ui/agent/blocks/UserBlock.vue'
import ModelMenu from '@demicodes/web-ui/agent/ModelMenu.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import { showToast } from '@demicodes/web-ui/infra/toast'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import { joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import { compactionTranscript, type CompactionCase } from '../fixtures/compaction'
import { RestoreSweep } from '../fixtures/restore-sweep'
import {
  sessionPaneStatus,
  type SessionLoad,
} from '@demicodes/web-ui/agent/session-status'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import UndeliveredRetryChip from '@demicodes/web-ui/agent/UndeliveredRetryChip.vue'
import { t as agentText } from '@demicodes/web-ui/agent/locales/text'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import AgentStatusBar from '@demicodes/web-ui/agent/AgentStatusBar.vue'
import RunEndTag from '@demicodes/web-ui/agent/blocks/RunEndTag.vue'
import TerminalChip from '@demicodes/web-ui/agent/TerminalChip.vue'
import TerminalPanel from '@demicodes/web-ui/agent/TerminalPanel.vue'
import { agentCompaction, firstRunningSubagentId } from '@demicodes/web-ui/agent/subagents'
import GalleryCachedSessions from '../components/GalleryCachedSessions.vue'
import GalleryTranscriptEntrance from '../components/GalleryTranscriptEntrance.vue'
import GalleryOpeningSteps from '../components/GalleryOpeningSteps.vue'
import GalleryHistoryWindow from '../components/GalleryHistoryWindow.vue'
import GalleryCommandWakeup from '../components/GalleryCommandWakeup.vue'
import GalleryBusyRequest from '../components/GalleryBusyRequest.vue'
import GalleryStreamingText from '../components/GalleryStreamingText.vue'
import GalleryConnectedSession from '../components/GalleryConnectedSession.vue'
import GalleryComments from '../components/GalleryComments.vue'
import GalleryAsks from '../components/GalleryAsks.vue'
import GalleryCommentBox from '../components/GalleryCommentBox.vue'
import GalleryPhoneFrame from '../components/GalleryPhoneFrame.vue'
import GalleryMessageEditing from '../components/GalleryMessageEditing.vue'
import GalleryAssistantMessages from '../components/GalleryAssistantMessages.vue'
import GalleryModelPreference from '../components/GalleryModelPreference.vue'
import GalleryFindBar from '../components/GalleryFindBar.vue'
import GalleryUserMessageLengths from '../components/GalleryUserMessageLengths.vue'
import { regenerateMessage, submitMessageEdit, type MessageEditHost, type MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import { useCallTerminals, firstRunningTerminalId, type TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { provideLiveCalls } from '@demicodes/web-ui/agent/live-calls'
import type { HostDeviceOption } from '@demicodes/web-ui/hosts/types'
import type { OfflineHost } from '@demicodes/web-ui/agent/offline-host'
import type { AgentMessage, UserContentBlock } from '@demicodes/protocol'
import { applyModelChange, type ModelSettings, type ModelSettingsChange } from '@demicodes/web-ui/agent/model-selection'
import { composerAttachment, encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import FunctionalBlock from '@demicodes/web-ui/agent/blocks/FunctionalBlock.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import {
  demoImageUrl,
  demoModel,
  changesDemoBlocks,
  editingShellTool,
  fileChangeCases,
  binaryStdoutTool,
  fullPageTool,
  longLineShellTool,
  longScriptShellTool,
  bashScriptShellTool,
  missingImageTool,
  notStoredVideoTool,
  recordingTool,
  runningShellTool,
  screenshotTool,
  screenshotsTool,
  fourScreenshotsTool,
  twelveScreenshotsTool,
  rowPictureSamples,
  screenshotRunBlocks,
  viewerTranscriptBlocks,
  shellTool,
  returnedShellTool,
  returnedFailingShellTool,
  smallImageTool,
  shortFramesTool,
  statusImageTool,
  thinkingText,
  steerPrompt,
  transcriptDemoBlocks,
  commandEndBlocks,
  commandReportCases,
  severalReports,
  removedToolBlocks,
  runningRunBlocks,
  steppedRunBlocks,
  STEPPED_RUN_STAGES,
  leftRunningBlocks,
  shellView,
  unsizedRecordingTool,
  wideImageTool,
} from '../fixtures/blocks'
import {
  demoProviders,
  offlineProviders,
  openaiOnlyProviders,
  usageAt,
} from '../fixtures/catalog'
import { composerModels, setGalleryContextLimit } from '../fixtures/context-limits'
import { galleryBlobs } from '../fixtures/blobs'
import { agentLinksWrapMarkdown, agentReceiptMessages, briefSubagent, gallerySubagents, mainAgentMessage, workSubagent } from '../fixtures/subagents'
import GalleryAgentDirectory from '../components/GalleryAgentDirectory.vue'
import GalleryAgentLoading from '../components/GalleryAgentLoading.vue'
import GalleryReceipts from '../components/GalleryReceipts.vue'
import { galleryTerminals, leftRunningCommand, runningRunCommand, steppedRunCommands } from '../fixtures/terminals'
import { demoDeviceStart } from '../fixtures/device-installation'
import { useLiveGalleryCommand } from '../live-command'
import { useTurnFlow, type TurnFlowKind } from '../turn-flow'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import GalleryAgentsChip from '../components/GalleryAgentsChip.vue'
import GalleryCommandReports from '../components/GalleryCommandReports.vue'
import GalleryComposer from '../components/GalleryComposer.vue'
import { notSavedOnTop, sevenSaved, twoSaved } from '../fixtures/saved'
import { noWorkflows, projectWorkflows } from '../fixtures/workflows'
import GalleryProjectStart from '../components/GalleryProjectStart.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GalleryContextLimit from '../components/GalleryContextLimit.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import GalleryTabStripDrive from '../components/GalleryTabStripDrive.vue'
import { useGalleryView } from '../gallery-views'
import GallerySharedPage from '../components/GallerySharedPage.vue'

const { view } = useGalleryView()
const historyModelBlocks = transcriptDemoBlocks()
/** The models the specimen's composer waits for have loaded: a message sent with Enter meanwhile goes then. */
const historyModelsLoaded = ref(false)
/** The calls the Command Ends specimens show, and calls of tools the runtime no longer has. */
const endBlocks = commandEndBlocks()
const removedBlocks = removedToolBlocks()
/** One report row specimen per event, and several reports in one block. */
const reportCases = commandReportCases()
const reportsTogether = severalReports()
/** One step of each kind, shown in each place it stands. */
const stepPlaceCases = stepPlaces()

/**
 * The permission card's specimens, each over its own requests: a decision
 * drops what the product's would (an allow every request of its category, a
 * deny that one) and says what the product would tell the agent; Show Again
 * brings the requests back.
 */
const permissionSpecimens = reactive<
  {
    variant: string
    fixture: () => RequestView[]
    requests: RequestView[]
    /** What the backend refused the last credential answer for; a new answer clears it, Show Again brings `refusal` back. */
    error: ShownError | null
    refusal: ShownError | null
  }[]
>([
  { variant: 'one request', fixture: () => [rootRequest()], requests: [rootRequest()], error: null, refusal: null },
  { variant: 'a queue of three', fixture: queuedRequests, requests: queuedRequests(), error: null, refusal: null },
  { variant: 'a subagent asked', fixture: () => [subagentRequest()], requests: [subagentRequest()], error: null, refusal: null },
  { variant: 'a host', fixture: () => [hostRequest()], requests: [hostRequest()], error: null, refusal: null },
  { variant: 'a pairing', fixture: () => [pairingRequest()], requests: [pairingRequest()], error: null, refusal: null },
  { variant: 'credential · one hidden field, kept for an hour', fixture: () => [googleKeyRequest()], requests: [googleKeyRequest()], error: null, refusal: null },
  { variant: 'credential · a visible and a hidden field, kept for 30 days', fixture: () => [signInRequest()], requests: [signInRequest()], error: null, refusal: null },
  { variant: 'credential · kept until the user removes it', fixture: () => [deployTokenRequest()], requests: [deployTokenRequest()], error: null, refusal: null },
  { variant: 'credential · renewal of a saved one', fixture: () => [renewalRequest()], requests: [renewalRequest()], error: null, refusal: null },
  { variant: 'credential · the values refused', fixture: () => [googleKeyRequest()], requests: [googleKeyRequest()], error: correction(INVALID_ANSWER), refusal: correction(INVALID_ANSWER) },
  { variant: 'credential · other fields than the saved ones', fixture: () => [renewalRequest()], requests: [renewalRequest()], error: correction(FIELDS_DIFFER), refusal: correction(FIELDS_DIFFER) },
  { variant: 'allowance', fixture: () => [allowanceRequest()], requests: [allowanceRequest()], error: null, refusal: null },
  { variant: 'allowance · the credential was removed', fixture: () => [removedAllowanceRequest()], requests: [removedAllowanceRequest()], error: null, refusal: null },
  {
    variant: 'a queue of all three kinds',
    fixture: () => [rootRequest(), googleKeyRequest(), allowanceRequest()],
    requests: [rootRequest(), googleKeyRequest(), allowanceRequest()],
    error: null,
    refusal: null,
  },
])
function decidePermission(
  requests: RequestView[],
  id: string,
  decision: PermissionDecision,
): RequestView[] {
  const decided = requests.find((request) => request.id === id)
  const kind = decided?.kind
  productWould(
    kind === 'credential'
      ? decision === 'allow'
        ? 'Save the Credential and Tell the Agent'
        : 'Tell the Agent the User Declined'
      : kind === 'allowance'
        ? decision === 'allow'
          ? 'Allow the Conversation to Use the Credential'
          : 'Tell the Agent the Use Was Not Allowed'
        : decision === 'deny'
          ? 'Tell the Agent the Request Was Denied'
          : decided?.kind === 'permission' && decided.host
            ? 'Let the Conversation Reach the Host and Tell Each Agent That Asked'
            : 'Allow the Categories and Tell Each Agent That Asked',
  )
  return afterDecision(requests, id, decision)
}
/** A receipt specimen's name: what the message is. */
function receiptVariant(message: AgentMessage): string {
  const event = message.event
  switch (event.type) {
    case 'message':
      return message.sender?.number === 0 ? 'the main agent’s message, in its child' : 'update'
    case 'moved':
      return 'moved, the agent told'
    case 'move_failed':
      return 'move failed'
    case 'permission':
      return event.action.includes(' and ') ? `${event.outcome}, two categories` : event.outcome
    default:
      return event.outcome
  }
}
/**
 * The dock's specimens: what stacks over the composer, one step apart, with
 * and without the card and the chips. A decision drops the card as the
 * product's would, and Show Again brings it back; a chip says what the
 * product would open.
 */
const dockSpecimens = reactive([
  { variant: 'card, no chips', asks: true, requests: [rootRequest()], chips: false, offline: false, device: false, host: 'Old Laptop' },
  { variant: 'card over chips', asks: true, requests: [rootRequest()], chips: true, offline: false, device: false, host: 'Old Laptop' },
  {
    variant: 'offline card over chips · Resume waits for the primary Host, and goes after a move',
    asks: false,
    requests: [] as RequestView[],
    chips: true,
    offline: true,
    device: true,
    host: 'Old Laptop',
  },
  {
    variant: 'permission card and offline card over chips',
    asks: true,
    requests: [rootRequest()],
    chips: true,
    offline: true,
    device: true,
    host: 'Old Laptop',
  },
  { variant: 'chips alone', asks: false, requests: [] as RequestView[], chips: true, offline: false, device: false, host: 'Old Laptop' },
])
// The product session opens with the root's request waiting, above its chips.
const sessionRequests = ref<RequestView[]>([rootRequest()])
// A first message the server never confirmed. Retry sends it again with the
// same id; this time it arrives and its turn runs.
const deliveryFlow = useTurnFlow({ id: 'gallery-delivery' })
const undeliveredText = `Review the notes ${ATTACHMENT_MARK} before the next run.`
const undeliveredContent = joinMessageContent(undeliveredText, [[{
  type: 'attachment',
  name: 'notes.txt',
  id: 'a1',
  mediaType: 'text/plain',
  sizeBytes: 2048,
  sha256: '0'.repeat(64),
} satisfies UserContentBlock]])

function failDelivery(): void {
  deliveryFlow.undelivered(
    undeliveredContent,
    { text: undeliveredText, attachments: [composerAttachment({ name: 'notes.txt', phase: 'ready' })] },
    // A cause of its own, worded as the backend's refusal names it.
    'The device is offline.',
  )
}
failDelivery()

const messageEdit = ref<MessageEditState | null>(null)
const editRevision = ref(0)
const agents = reactive(gallerySubagents())
const terminals = reactive(galleryTerminals())
// The running call's command prints as it runs; the call's specimen shows it
// under the call, and the Terminals window in its tab.
useLiveGalleryCommand(terminals)
const finishedOnly = agents.filter((agent) => agent.phase !== 'running')
const exhibitOpen = ref(true)
const exhibitAgentId = ref<string | null>(firstRunningSubagentId(agents))
const exhibitTerminalId = ref<string | null>(firstRunningTerminalId(terminals))
/** One child per variant of the line that says what a child runs on. */
const runLines = [workSubagent(), ...gallerySubagents()]
/** What the children's context cards show and do beside their usage. */
const agentContext = useAgentContext()
const runLineVariants: Record<string, string> = {
  'ag-suite': 'running · profile',
  'ag-cookie': 'running · no profile',
  'ag-logout': 'running · Fast · forked',
  'ag-snapshot': 'running · thinking off',
  'ag-comments': 'finished · no profile',
  'ag-fixture': 'failed',
  'ag-ci': 'stopped',
}
const briefAgents = reactive([briefSubagent()])
const briefOpen = ref(true)
const briefAgentId = ref<string | null>(briefAgents[0]!.id)
/** Stop Agent or Stop All on the brief's child: it stops, and its tab stays while the window shows it. */
function stopBriefAgent(): void {
  const agent = briefAgents[0]!
  if (agent.phase === 'running') {
    agent.phase = 'aborted'
    agent.endedAt = new Date().toISOString()
  }
}
/** Run Them Again: the windows' agents and jobs run again as the fixtures start them, each window open on its first. */
function resetWindows(): void {
  const freshAgents = gallerySubagents()
  for (const agent of agents) {
    const fresh = freshAgents.find((entry) => entry.id === agent.id)
    if (fresh) {
      agent.phase = fresh.phase
      agent.endedAt = fresh.endedAt
    }
  }
  const freshTerminals = galleryTerminals()
  for (const terminal of terminals) {
    const fresh = freshTerminals.find((entry) => entry.id === terminal.id)
    if (fresh) {
      terminal.phase = fresh.phase
      terminal.endedAt = fresh.endedAt
      delete terminal.stopping
    }
  }
  exhibitOpen.value = true
  exhibitAgentId.value = firstRunningSubagentId(agents)
  exhibitTerminalId.value = firstRunningTerminalId(terminals)
}
// The Agents panel open while a request waits: the card stands over the panel's lower part.
const coverRequests = ref<RequestView[]>([rootRequest()])
const coverOpen = ref(true)
const coverAgentId = ref<string | null>(firstRunningSubagentId(agents))
const coverBlocks = transcriptDemoBlocks()
const coverSurface = ref<{ dockHeight: number }>()
/** The returned call's command, a Running job, which End Command stops. */
const returnedCommand = computed(() => terminals.find((terminal) => terminal.toolUseId === returnedShellTool.toolUseId))
/** The other returned call's command, which End Command ends with exit code 1. */
const returnedFailingCommand = reactive<TerminalRecord>({
  id: 'term-typecheck',
  title: 'Watch the type check',
  script: 'bun run typecheck --watch',
  phase: 'running',
  startedAt: new Date().toISOString(),
  output: 'Watching for changes…\n',
  toolUseId: returnedFailingShellTool.toolUseId,
})

/** How long the gallery's runner takes to say that a stopped command ended, while it shows Stopping…. */
const STOPPING_MS = 3000
/** The stops waiting for that word. */
const stoppingTimers = new Set<number>()
onBeforeUnmount(() => stoppingTimers.forEach((timer) => window.clearTimeout(timer)))

/**
 * Ends a returned call's command as its last frames would, `end` saying how,
 * or runs it again. A stop shows the command stopping until its runner, simulated
 * here, says that it ended.
 */
function toggleReturnedCommand(command: TerminalRecord | undefined, end: Pick<TerminalRecord, 'phase' | 'exitCode'>): void {
  if (!command || command.stopping) {
    return
  }
  if (command.phase === 'running' && end.phase === 'aborted') {
    command.stopping = true
    const timer = window.setTimeout(() => {
      stoppingTimers.delete(timer)
      delete command.stopping
      Object.assign(command, end, { endedAt: new Date().toISOString() })
    }, STOPPING_MS)
    stoppingTimers.add(timer)
    return
  }
  if (command.phase === 'running') {
    Object.assign(command, end, { endedAt: new Date().toISOString() })
    return
  }
  command.phase = 'running'
  command.startedAt = new Date().toISOString()
  delete command.exitCode
  delete command.endedAt
}
const fillPane = computed(() => view.value === 'session')
const editVersion = computed(() => ({ epoch: 'gallery-session', revision: editRevision.value }))

watch(
  view,
  (next) => {
    if (next !== 'windows') {
      return
    }
    exhibitOpen.value = true
    exhibitAgentId.value = firstRunningSubagentId(agents)
    exhibitTerminalId.value = firstRunningTerminalId(terminals)
  },
  { immediate: true },
)
// The Panel view: the whole app frame, with the work panel open beside the session.
const panelSidebarWidth = ref<number>(SIDEBAR_WIDTH.default)
const panelAsideShare = ref<number>(ASIDE_SHARE.default)
const panelProjects = ref(demoProjects())
const panelConversations = ref(demoConversations())
const panelActiveConversationId = ref<string | null>('c-login')
/** The frame's search window, over the sidebar's conversations and the session's messages, which belong to c-login. */
const frameSearchOpen = ref(false)
/** The message a search result opened the session at, until the session has shown it. */
const frameReveal = ref<TranscriptReveal | null>(null)
/** The block a receipt's name in the Session view asked to bring into view, until the session has shown it. */
const sessionReveal = ref<TranscriptReveal | null>(null)
const frameSearchable = computed(() =>
  panelConversations.value.map((conversation) => ({
    conversationId: conversation.id,
    title: conversation.title,
    archived: false,
    lastActiveAt: conversation.updatedAt,
    messages: conversation.id === 'c-login' ? searchableMessages(session.blocks) : [],
  })),
)
const frameRecent = computed<SearchRow[]>(() =>
  frameSearchable.value
    .map(({ messages: _messages, ...row }) => ({ ...row, titleRanges: [], match: null }))
    .toSorted((a, b) => b.lastActiveAt.localeCompare(a.lastActiveAt)),
)
const frameSearch = fixtureSearch(() => frameSearchable.value)
/** Opens what the search found, as the product does: the conversation, at the message that matched. */
function openFrameResult(row: SearchRow): void {
  frameSearchOpen.value = false
  panelActiveConversationId.value = row.conversationId
  frameReveal.value = row.match ? { node: null, blockId: row.match.blockId } : null
}
/**
 * One specimen's work panel over the gallery workspace, as the product's work
 * store holds one: the plugins' kinds, with the `browser` kind over the
 * gallery's own conversation browser or the one the specimen supplies, and
 * the File view on `path` when one is given. A specimen can also say whether
 * the web browser decodes the pictures.
 */
function useWorkTabs(
  selection: string,
  { path = 'src/auth/cookie.ts', browser = galleryBrowser(), pictures }: {
    path?: string
    browser?: GalleryBrowser
    pictures?: () => Promise<boolean>
  } = {},
) {
  const work = useGalleryWork(selection, { files, browser, pictures })
  if (path) {
    work.openIn({ intent: 'file', payload: { path: `${workspace.root}/${path}` } })
    work.select(selection)
  }
  return work
}
const workspace = createGalleryWorkspace()
const files = galleryFiles(workspace)
/** The same workspace on a Host that cannot watch its files. */
const unwatchedWorkspace = createGalleryWorkspace(200, 'unavailable')
/** The same workspace once more, for a refresh that fails. */
const failingWorkspace = createGalleryWorkspace()
const COOKIE = 'src/auth/cookie.ts'
const cookiePath = `${workspace.root}/${COOKIE}`
let outsideEdits = 0

/**
 * Simulates a program outside Demi writing cookie.ts on the Host: the
 * simulated watch reports it, and the live specimen, shown on the file,
 * reads it again and replaces its text in place.
 */
async function editCookieOutside(): Promise<void> {
  showInFileView(cookiePath)
  const text = await workspace.source.read(cookiePath)
  outsideEdits += 1
  workspace.source.change(cookiePath, `${text}\n// Edited outside Demi (${outsideEdits})\n`)
  productWould('The Host Reported cookie.ts Changed')
}

/** Simulates the Host failing the next read of cookie.ts, and a report that has it read again. */
async function failCookieRefresh(): Promise<void> {
  const text = await failingWorkspace.source.read(cookiePath)
  failingWorkspace.source.failNext(cookiePath, { kind: 'other', message: 'The Host did not answer in time.' })
  failingWorkspace.source.change(cookiePath, text)
  productWould('The Host Reported cookie.ts Changed; Reading It Again Failed')
}
const fileViewTree = ref(true)
const changeViewTree = ref(true)
const changeViewCommitsHeight = ref(COMMITS_HEIGHT.default)
const fileViewMode = ref<'preview' | 'source'>('preview')
const changeViewPresentation = ref<'diff' | 'preview'>('diff')
/** One of each kind the File view previews, by workspace path. */
const previewFiles = [
  'README.md', 'assets/logo.svg', 'assets/photo.png', 'assets/demo.mp4',
  'assets/tone.m4a', 'docs/guide.pdf', 'dist/app.zip', 'src/auth/cookie.ts', 'server.log',
  'packages/agent/src/providers/anthropic.ts',
]
/**
 * One Change view on its own, stepped the way the `change` kind steps it:
 * for the Change view specimens. `request` opens a request of the
 * conversation, as its line or a pill would, whose files `conversation` holds.
 */
function useChangeTab(
  mode: ChangeMode,
  path: string | null,
  changes: { uncommitted: ChangeSetSource; conversation: RequestChangeSource | null; history?: CommitHistorySource | null },
  request: RequestEditSelection | null = null,
  /** What the tab shows next in Commits, such as a commit picked or a link followed. */
  then: (data: ChangeData) => ChangeData = (data) => data,
) {
  const tab = ref(then(request ? showRequestEdit(firstChangeData(), request) : showChange(firstChangeData(), mode, path)))
  /** The file the view holds in its mode, else the first there is, which it then holds. */
  const selected = computed(() => changePath(tab.value, tab.value.mode, changes.uncommitted.files))
  watch(selected, () => {
    tab.value = holdShownChange(tab.value, changes.uncommitted.files)
  }, { immediate: true })
  function show(mode: ChangeMode, path: string | null) {
    tab.value = showChange(tab.value, mode, path)
  }
  return {
    tab,
    selected,
    changes: computed<ChangeSources>(() => ({
      uncommitted: changes.uncommitted,
      history: changes.history ?? null,
      conversation: tab.value.request ? changes.conversation : null,
    })),
    edit: computed(() => tab.value.request?.edit ?? null),
    setMode: (mode: ChangeMode) => show(mode, changePath(tab.value, mode)),
    select: (path: string | null) => show(tab.value.mode, path),
    setEdit: (edit: RequestEditRef | null) => (tab.value = showEdit(tab.value, edit)),
    back: () => (tab.value = changeBack(tab.value)),
    forward: () => (tab.value = changeForward(tab.value)),
    setCommit: (commit: string | null) => (tab.value = showItem(tab.value, commit)),
    setBranch: (branch: string | null) => (tab.value = showBranch(tab.value, branch)),
    placed: (branch: string | null) => (tab.value = placeBranch(tab.value, branch)),
  }
}
const fileViewPath = ref(`${workspace.root}/src/auth/cookie.ts`)
const fileViewBack = ref<string[]>([])
const fileViewForward = ref<string[]>([])
function showInFileView(path: string) {
  fileViewBack.value = [...fileViewBack.value, fileViewPath.value]
  fileViewForward.value = []
  fileViewPath.value = path
}
function fileViewGoBack() {
  const previous = fileViewBack.value.at(-1)
  if (previous === undefined) return
  fileViewBack.value = fileViewBack.value.slice(0, -1)
  fileViewForward.value = [...fileViewForward.value, fileViewPath.value]
  fileViewPath.value = previous
}
function fileViewGoForward() {
  const next = fileViewForward.value.at(-1)
  if (next === undefined) return
  fileViewForward.value = fileViewForward.value.slice(0, -1)
  fileViewBack.value = [...fileViewBack.value, fileViewPath.value]
  fileViewPath.value = next
}
/** The reply that quotes a Rust file and a Bash script whole, beside the File view showing either. */
const quotedReply = [
  `The port comes from \`DEMI_PORT\` in \`${QUOTED_CODE.rust.path}\`:`,
  '',
  '```rust',
  QUOTED_CODE.rust.text.trimEnd(),
  '```',
  '',
  `and \`${QUOTED_CODE.bash.path}\` passes it on:`,
  '',
  '```bash',
  QUOTED_CODE.bash.text.trimEnd(),
  '```',
].join('\n')
const quotedFiles = [QUOTED_CODE.rust.path, QUOTED_CODE.bash.path].map((path) => `${workspace.root}/${path}`)
const quotedPath = ref(quotedFiles[0]!)
const quotedTree = ref(false)
/** The narrow File view that has shown no file yet: a pick in its tree opens one, and Back returns to none. */
const unchosenPath = ref<string | null>(null)
const unchosenTree = ref(true)
// The frame's conversation has opened no browser tab yet: its strip starts empty.
const panelWork = useWorkTabs('change', { browser: galleryBrowser([]) })
/** Whether the frame's panel is open: the user closes it, and a tab the agent shows opens it. */
const panelAsideOpen = panelWork.open
/** The agent opens a page, showing it to the user or checking it itself, as `demi browser open [--show]` does. */
function agentOpens(show: boolean) {
  productWould(show ? 'The Agent Opens a Page and Shows It' : 'The Agent Opens a Page to Check It')
  void panelWork.agentOpens(show ? 'https://example.test/orders' : 'https://example.test/docs', show)
}
/** The session's messages reach the gallery workspace: images from its fixtures, files opened in the frame's panel. */
const sessionFiles = galleryConversationFiles(workspace.source, ({ path, line }) => {
  // From the session page, which shows no panel, a click opens the frame's.
  panelWork.openIn({ intent: 'file', payload: line === null ? { path } : { path, line } })
  view.value = 'panel'
}, ({ commit, root }) => {
  panelWork.openIn({ intent: 'commit', payload: { commit, ...(root === undefined ? {} : { root }) } })
  view.value = 'panel'
}, ({ path }) => {
  panelWork.openIn({ intent: 'file', payload: { path, tree: true } })
  view.value = 'panel'
}, (id, { name, size, url }) => {
  panelWork.openIn({ intent: 'attachment', payload: { id, name, size, url } })
  view.value = 'panel'
})
// The tabs specimen starts on an empty strip, which only the agent's tabs fill.
const exhibitWork = useWorkTabs('file', { browser: galleryBrowser([]) })
/** The agent opens a page and shows it in the tabs specimen, as `demi browser open --show` does. */
function exhibitAgentOpens() {
  productWould('The Agent Opens a Page and Shows It')
  void exhibitWork.agentOpens('https://example.test/orders', true)
}
/** The simulators specimen: a strip with no browser tab, where the agent opens devices. */
const mobileWork = useWorkTabs('file', { browser: galleryBrowser([]) })
const editWork = useWorkTabs('change')
// The gallery's own conversation browser stands behind every specimen's `browser`
// kind, which lists its tabs as it is made.
provideEditSelection(() => editWork.selectEdit)
const gitHistory = sharedHistory.source
const changeUncommitted = useChangeTab('commits', 'src/auth/cookie.ts', { uncommitted: workspace.changes, conversation: null, history: gitHistory })
/** Commits on another branch than the one checked out, its newest commit picked. */
const changeOtherBranch = useChangeTab('commits', null, { uncommitted: workspace.changes, conversation: null, history: gitHistory }, null, (data) => showItem(showBranch(data, 'feat/cart'), sharedHistory.ids.cart))
/** A commit of the branch checked out picked in the list. */
const changeCommit = useChangeTab('commits', null, { uncommitted: workspace.changes, conversation: null, history: gitHistory }, null, (data) => showItem(data, sharedHistory.ids.latest()))
/** A link to a commit that no branch holds, as a rebase leaves one. */
const changeNoBranch = useChangeTab('commits', null, { uncommitted: workspace.changes, conversation: null, history: gitHistory }, null, (data) => showCommit(data, { commit: sharedHistory.ids.orphan.slice(0, 7) }))
/** A link to a commit the repository does not have. */
const changeMissingCommit = useChangeTab('commits', null, { uncommitted: workspace.changes, conversation: null, history: gitHistory }, null, (data) => showCommit(data, { commit: 'deadbee' }))
/** Makes a commit on the branch checked out, as a terminal would; every Commits specimen's list follows. */
function commitInTerminal(): void {
  sharedHistory.commit('Tidy the cart styles')
  productWould('A Commit Made in a Terminal Shows in the List')
}
/** The sign-in request, its login.ts edited three times, as the Change view's specimens read it. */
const signInChanges = storedRequests(signInRequestBlocks('change-view')).requests[0]!
const signInLogin = signInChanges.files[0]!
const changeRequest = useChangeTab('conversation', null, {
  uncommitted: workspace.changes,
  conversation: { files: signInChanges.files, read: readGalleryEdit },
}, { node: null, request: signInChanges.id, file: signInLogin.path, edit: null })
const changeRequestEdit = useChangeTab('conversation', null, {
  uncommitted: workspace.changes,
  conversation: { files: signInChanges.files, read: readGalleryEdit },
}, { node: null, request: signInChanges.id, file: signInLogin.path, edit: signInLogin.edits[2]! })
/** A request that changed the README once, whose two sides are the workspace's Markdown. */
const readmeChange = editedFile({ path: 'README.md', kind: 'modified', added: 0, removed: 1 })
const changeDocument = useChangeTab('conversation', null, {
  uncommitted: workspace.changes,
  conversation: {
    files: [{
      path: readmeChange.path,
      kind: readmeChange.kind,
      lines: { added: readmeChange.added, removed: readmeChange.removed },
      edits: [{ call: 'gallery-readme-edit', title: 'Tidy the readme', path: readmeChange.path, segment: 0, copies: readmeChange.edits[0]!.copies }],
    }],
    read: () => gallerySides('README.md'),
  },
}, { node: null, request: 'gallery-readme', file: 'README.md', edit: null })
/**
 * A request that removed a file it never changed, so nothing of it was
 * kept, beside the readme it edited: the removed file is picked.
 */
const changeRemoved = useChangeTab('conversation', null, {
  uncommitted: workspace.changes,
  conversation: {
    files: [
      {
        path: readmeChange.path,
        kind: readmeChange.kind,
        lines: { added: readmeChange.added, removed: readmeChange.removed },
        edits: [{ call: 'gallery-tidy', title: 'Tidy the docs', path: readmeChange.path, segment: 0, copies: readmeChange.edits[0]!.copies }],
      },
      { path: 'docs/old-guide.md', kind: 'deleted', edits: [] },
    ],
    read: () => gallerySides('README.md'),
  },
}, { node: null, request: 'gallery-tidy', file: 'docs/old-guide.md', edit: null })
/**
 * A request that made a picture and packed a release beside the readme it
 * edited: a binary edit keeps no contents, so each says why and opens the
 * file, whatever its name says it is.
 */
const changeBinary = useChangeTab('conversation', null, {
  uncommitted: workspace.changes,
  conversation: {
    files: [
      {
        path: readmeChange.path,
        kind: readmeChange.kind,
        lines: { added: readmeChange.added, removed: readmeChange.removed },
        edits: [{ call: 'gallery-hero', title: 'Draw the hero image', path: readmeChange.path, segment: 0, copies: readmeChange.edits[0]!.copies }],
      },
      { path: 'assets/hero.png', kind: 'added', edits: [{ call: 'gallery-hero', title: 'Draw the hero image', path: 'assets/hero.png', segment: 0, created: true, binary: true }] },
      { path: 'dist/release', kind: 'added', edits: [{ call: 'gallery-hero', title: 'Pack the release', path: 'dist/release', segment: 0, created: true, binary: true }] },
    ],
    read: () => gallerySides('README.md'),
  },
}, { node: null, request: 'gallery-hero', file: 'assets/hero.png', edit: null })
const changeEmpty = useChangeTab('conversation', null, { conversation: null, uncommitted: workspace.changes })
const changeNoRepository = useChangeTab('commits', null, {
  conversation: null,
  uncommitted: createGalleryChangeSet(200, { unavailable: 'no-repository' }),
})
// A commit link opened in a folder outside any repository.
const changeNoRepositoryCommit = useChangeTab('commits', null, {
  conversation: null,
  uncommitted: createGalleryChangeSet(200, { unavailable: 'no-repository' }),
  history: gitHistory,
}, null, (data) => showCommit(data, { commit: LINKED_COMMIT }))
const changeStale = useChangeTab('commits', 'src/auth/cookie.ts', {
  conversation: null,
  uncommitted: createGalleryChangeSet(200, { truncated: true, failure: 'The device is offline.' }),
})
// The conversation browser's own tabs.
const browserWork = useWorkTabs('change', { path: '' })
/** The browser tab the specimen shows, as its kind reads it. */
const shownBrowserTab = computed(() => {
  const tab = selectedTab(browserWork.panel.value, browserWork.kinds)
  const parsed = tab?.kind === 'browser' ? browserTabDataSchema.safeParse(tab.data) : null
  return parsed?.success ? (parsed.data.tab ?? null) : null
})
/** The browser tab the panel does not show that the agent shows next, as `demi browser show` would. */
const toShow = computed(() => browserWork.browser.listed.value.tabs.find((tab) => tab.id !== shownBrowserTab.value) ?? null)
/** The agent shows that tab: once its job ends, the panel selects it, once per showing. */
function agentShows() {
  const tab = toShow.value
  if (tab === null) {
    return
  }
  productWould(`The Agent Shows ${tab.title || tab.id}`)
  void browserWork.agentShows(tab.id)
}
/** Closes the page behind the panel's back, as the agent's `close` would; the view finds it gone and the plugin removes its tab. */
function closeOnDevice() {
  const tab = shownBrowserTab.value
  if (tab !== null) {
    browserWork.browser.closeOnDevice(tab)
  }
}
/** Ends the conversation's browser, as a release or a stopped Cloud does: the shown tab opens again at once. */
function endBrowser() {
  browserWork.browser.end()
}
/** Stops the Cloud, as an idle Cloud stops: the shown tab opens again at once, starting the Cloud first. */
function stopCloud() {
  browserWork.browser.stopCloud()
  productWould('The Cloud Stops; Its Browser Ends')
}
/**
 * A tab opened while the browser does not have it yet, held in one phase:
 * the page connecting, the Cloud starting, the browser starting on a Host
 * whose browser has no tabs, or the browser opening the tab, until the
 * specimen lets it finish.
 */
/** A tab whose browser tab the browser lost, shown, so it reopens on its address at once, as after a restart. */
function addLostTab(work: GalleryWork) {
  work.lost('https://github.com/login')
}
function heldWork(phase: StartingPhase) {
  const work = useWorkTabs('change', { path: '', browser: galleryBrowser(phase === 'page' || phase === 'connecting' ? undefined : [], { held: phase }) })
  addLostTab(work)
  return work
}
const STARTING_PHASES: readonly StartingPhase[] = ['connecting', 'cloud', 'browser', 'page']
const heldWorks: Record<StartingPhase, GalleryWork> = {
  connecting: heldWork('connecting'),
  cloud: heldWork('cloud'),
  browser: heldWork('browser'),
  page: heldWork('page'),
}
/** The Host goes on: the held tab opens. */
function finishStarting(phase: StartingPhase) {
  heldWorks[phase].browser.finish()
  productWould('The Host Goes On; the Tab Opens')
}
/** Another lost tab that waits in the same phase as it reopens, as the specimen started. */
function holdAgain(phase: StartingPhase) {
  heldWorks[phase].browser.hold(phase)
  addLostTab(heldWorks[phase])
}
/** The device refuses the next tab it would open, as an offline device does: a reopening shows Retry. */
function failNextOpen() {
  browserWork.browser.failNextOpen(BrowserTabsError.of('device_offline'))
  productWould('The Device Refuses the Next Tab It Opens')
}
/** The browser cannot start on the device for the next tab, as when its package or Chrome fails to start. */
function failNextStart() {
  browserWork.browser.failNextOpen(BrowserTabsError.of('browser_unavailable'))
  productWould('The Browser Fails to Start for the Next Tab')
}
/** The server fails the next tab without a reason of the browser's, as a defect of the plugin would. */
function failNextOnServer() {
  browserWork.browser.failNextOpen(new BrowserTabsError({ status: SERVER_FAILED }))
  productWould('The Server Fails the Next Tab')
}
// The same conversation browser, viewed in a web browser that cannot decode H.264, such as a Chromium without
// proprietary codecs.
const undecodedWork = useWorkTabs('change', { path: '', pictures: async () => false })
// A Host that cannot capture its tabs, such as an Apple M4's Linux VM: the agent's tab shows why it has no picture.
const uncapturedWork = useWorkTabs('browser-t1', { path: '', browser: galleryBrowser(undefined, { capture: false }) })
/** What the Host's live view module does when it cannot deliver the viewer's input: it tells the viewer in a notice. */
function failInput() {
  browserWork.browser.fail('driver_error', 'Input.dispatchKeyEvent: the target closed')
}
/** The Host cannot even recreate its capture extension: the view stops trying, and its Retry asks again. */
function stopCapture() {
  browserWork.browser.stopCapture()
  // What it does shows on a browser tab: a specimen showing another kind shows its first browser tab.
  if (shownBrowserTab.value === null) {
    const tab = browserWork.panel.value.tabs.find((candidate) => candidate.kind === 'browser')
    if (tab) {
      browserWork.select(tab.id)
    }
  }
}
// A conversation its first send has not created: no page is bound beside it.
const unstartedWork = useGalleryWork(null, { files, pages: [] })
// The Session view is the product's ChatSession over a scripted runtime; Turns and Stream replay one flow each.
const sessionFlow = useTurnFlow({
  id: 'gallery-session',
  title: 'Login test',
  cwd: workspace.root,
  blocks: transcriptDemoBlocks(),
  subagents: agents,
  terminals,
})
const session = sessionFlow.state
/** The conversation's shares and the service that makes them, as the product's header menu and answer footers use them. */
const { shares: sessionShares, service: shareService } = useGalleryShares()
session.pendingSteers = [{ id: 'pending-1', content: steerPrompt }]
session.queue = [
  { id: 'q1', content: [{ type: 'text', text: 'Also add a case for the expired cookie.' }] },
  { id: 'q2', content: [{ type: 'text', text: 'Keep the light-mode screenshot in the same PR.' }] },
]
const streamFlow = useTurnFlow({ id: 'gallery-stream' })
const turnFlow = useTurnFlow({ id: 'gallery-turn' })
const workFlow = useTurnFlow({ id: 'gallery-work', subagents: [workSubagent()] })
workFlow.settleWork()
/**
 * A running turn whose run's newest call runs a command: End Command ends
 * it as its last frame and the call's result would, exited with 0; Run
 * Again starts it anew, so its tag shows again past 3 seconds.
 */
const runningRun = reactive({ blocks: runningRunBlocks(), command: runningRunCommand() })
function toggleRunningRun(): void {
  const index = runningRun.blocks.length - 1
  const call = runningRun.blocks[index]
  if (call?.type !== 'tool_call') {
    return
  }
  const command = runningRun.command
  if (command.phase === 'running') {
    const endedAt = new Date().toISOString()
    Object.assign(command, { phase: 'exited', exitCode: 0, endedAt })
    // The list is replaced, as the page's store replaces it, never changed in place.
    runningRun.blocks = runningRun.blocks.with(index, {
      ...call,
      status: 'completed',
      view: shellView({ commandId: '62', runningMs: Date.parse(endedAt) - Date.parse(command.startedAt), chunks: [{ stream: 'stdout', text: command.output }] }),
    })
    return
  }
  Object.assign(command, { phase: 'running', startedAt: new Date().toISOString() })
  delete command.exitCode
  delete command.endedAt
  runningRun.blocks = runningRun.blocks.with(index, { ...call, status: 'executing', view: null })
}
/**
 * A run stepped through what the transcript shows while it goes: Next Step
 * brings the stage of `STEPPED_RUN_STAGES` after it, and the last, the
 * answer, is followed by the first again.
 */
const steppedStage = ref(0)
const steppedBlocks = computed(() => steppedRunBlocks(steppedStage.value))
const steppedLast = computed(() => steppedStage.value === STEPPED_RUN_STAGES.length - 1)
function nextStep(): void {
  steppedStage.value = steppedLast.value ? 0 : steppedStage.value + 1
}
/**
 * An ended turn that left its command running: End Command ends it as its
 * last frame would, exited with 0, and the run's tag goes; Run Again
 * starts it anew.
 */
const leftRunning = reactive({ blocks: leftRunningBlocks(), command: leftRunningCommand() })
function toggleLeftRunning(): void {
  const command = leftRunning.command
  if (command.phase === 'running') {
    Object.assign(command, { phase: 'exited', exitCode: 0, endedAt: new Date().toISOString() })
    return
  }
  Object.assign(command, { phase: 'running', startedAt: new Date().toISOString() })
  delete command.exitCode
  delete command.endedAt
}
// The specimens' calls show their commands' output while they run: the live
// call's and the Turn's, whose flow runs its own command.
const commandOf = useCallTerminals(() =>
  [...terminals, ...turnFlow.state.terminals, ...workFlow.state.terminals, returnedFailingCommand, runningRun.command, leftRunning.command, ...steppedRunCommands(steppedStage.value)])
provideLiveCalls((toolUseId) => commandOf(undefined, toolUseId))
/** The devices an offline primary Host's card offers, as Run On lists them: it shows the online ones. */
const offlineMenuDevices: HostDeviceOption[] = shownAmong([
  { id: 'mac', name: 'MacBook Pro', state: 'online', path: 'thisComputer', mark: GALLERY_MARKS.mac },
  { id: 'studio', name: 'Studio PC', state: 'online', path: 'localNetwork', mark: GALLERY_MARKS.windows },
  { id: 'old', name: 'Old Laptop', state: 'offline', mark: GALLERY_MARKS.debian },
  { id: 'build', name: 'build-box', state: 'offline', mark: GALLERY_MARKS.ubuntu },
])
/** A dock specimen's primary Host while it is offline: the card names it, and offers the others. */
function dockOfflineHost(name: string): OfflineHost {
  const id = offlineMenuDevices.find((device) => device.name === name)?.id ?? 'old'
  return {
    name,
    start: demoDeviceStart('macos'),
    primaryHost: { id, name, kind: 'device', state: 'offline', mark: offlineMenuDevices.find((device) => device.id === id)?.mark ?? null },
    devices: offlineMenuDevices.map((device) => device.id === id ? { ...device, state: 'offline' } : device),
  }
}

// The frame's and the Session view's pills open their requests in the frame's panel.
useGalleryTranscripts(() => ({
  blocks: session.blocks,
  subagents: session.subagents.map((agent) => ({ id: agent.id, blocks: latestBlocks(agent.history) })),
}))
const changesFlow = useTurnFlow({ id: 'gallery-changes', title: 'Cookie rename', blocks: changesDemoBlocks() })
useGalleryTranscripts(() => ({ blocks: changesFlow.state.blocks, subagents: [] }))
useGalleryTranscripts(() => ({ blocks: workFlow.state.blocks, subagents: [] }))
/** The requests of the Request’s Changes specimens, which the panel under them reads as the product reads a conversation. */
const signInBlocks = signInRequestBlocks('sign-in')
const signInOffBlocks = signInRequestBlocks('sign-in-off')
const signInOutsideBlocks = signInRequestBlocks('sign-in-outside')
/** The same request still working, its last call ended and no reply yet. */
const signInRunningBlocks = signInRequestBlocks('sign-in-running').slice(0, -1)
// The calls being written: a finished sentence, then a call with its title,
// and one without it yet, where Requesting follows the text as when no call
// is being written.
const writingBlocks = callBeingWrittenBlocks()
// Runs that only check a command, and ones that run one too: their rows name what they did, never a count of steps.
const lookBlocks = commandLookBlocks()
// A run that viewed screenshots, folded with them under its row, and a report that carried some.
const viewedBlocks = viewedMediaBlocks()
const screenshotRun = screenshotRunBlocks()
// A transcript whose pictures the media viewer steps through with the arrow keys.
const viewerTranscript = viewerTranscriptBlocks()
const writingSpecimens = [
  { variant: 'being written, with its description', calls: [{ toolUseId: 'writing-1', toolName: 'shell', description: 'Write the categorizer' }] },
  { variant: 'without a description yet', calls: [{ toolUseId: 'writing-1', toolName: 'shell', description: null }] },
]
const uncopiedBlocks = uncopiedRequestBlocks()
const movedBlocks = movedRequestBlocks()
const cleanupBlocks = cleanupRequestBlocks()
const parentBlocks = subagentParentBlocks()
const childBlocks = subagentBlocks()
useGalleryTranscripts(() => ({ blocks: signInBlocks, subagents: [] }))
useGalleryTranscripts(() => ({ blocks: signInOutsideBlocks, subagents: [] }))
useGalleryTranscripts(() => ({ blocks: uncopiedBlocks, subagents: [] }))
useGalleryTranscripts(() => ({ blocks: movedBlocks, subagents: [] }))
useGalleryTranscripts(() => ({ blocks: cleanupBlocks, subagents: [] }))
useGalleryTranscripts(() => ({ blocks: parentBlocks, subagents: [{ id: SUBAGENT, blocks: childBlocks }] }))
const changesSurface = ref<{ dockHeight: number }>()
const changesList = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()
const turnSurface = ref<{ dockHeight: number }>()
const turnList = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()
const workSurface = ref<{ dockHeight: number }>()
const workList = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()
const streamSurface = ref<{ dockHeight: number }>()
const streamList = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()

function playStream(): void {
  streamFlow.play('stream')
}

function playTurn(kind: TurnFlowKind): void {
  turnFlow.play(kind)
}

/** The catalog with the user's context limits, which the selector specimens' Context rows change. */
const selectorSettings = ref<ModelSettings>({
  providerId: 'anthropic',
  modelId: 'claude-sonnet',
  thinkingEffort: 'medium',
  serviceTierId: null,
})
const fastSettings = ref<ModelSettings>({
  providerId: 'anthropic',
  modelId: 'claude-sonnet',
  thinkingEffort: 'medium',
  serviceTierId: 'priority',
})

function changeSelector(change: ModelSettingsChange): void {
  selectorSettings.value = applyModelChange(selectorSettings.value, change)
}

function changeFast(change: ModelSettingsChange): void {
  fastSettings.value = applyModelChange(fastSettings.value, change)
}
const pastedSnippet = `2026-09-12T10:41:02Z INFO  auth  session cookie renamed sid -> session
2026-09-12T10:41:02Z WARN  auth  legacy cookie still read by login test
2026-09-12T10:41:03Z ERROR test  expect(received).toBe(expected)
  Expected: "session"
  Received: "sid"
2026-09-12T10:41:03Z INFO  test  1 fail 2 pass 1 skip`
const markdownSnippet = `# Release notes · 0.9
- Session cookie renamed to \`session\`
- Login page keeps both fields on a rejected sign-in
- Terminal tabs close with the running command`
/** A sent message with its files where the user put them: a record for each, the picture before an image's. */
const attachmentBubble: UserContentBlock[] = [
  { type: 'text', text: 'The spec is ' },
  {
    type: 'document',
    source: {
      type: 'ref',
      ref: galleryBlobs.guide,
      mediaType: 'application/pdf',
      fileName: 'login-failure.pdf',
    },
  },
  {
    type: 'attachment',
    name: 'login-failure.pdf',
    id: 'a2',
    mediaType: 'application/pdf',
    sizeBytes: 120334,
    sha256: 'demo-pdf',
  },
  { type: 'text', text: ', the screenshot from CI is ' },
  { type: 'image', source: { type: 'ref', ref: galleryBlobs.screenshot, mediaType: 'image/png', width: 480, height: 300 } },
  {
    type: 'attachment',
    name: 'login-fail.png',
    id: 'a3',
    mediaType: 'image/png',
    sizeBytes: 48211,
    sha256: 'demo-png',
  },
  { type: 'text', text: ', the recording of it is ' },
  { type: 'video', source: { type: 'ref', ref: galleryBlobs.recording, mediaType: 'video/webm', width: 320, height: 180 } },
  {
    type: 'attachment',
    name: 'login-fail.webm',
    id: 'a4',
    mediaType: 'video/webm',
    sizeBytes: 23336,
    sha256: 'demo-webm',
  },
  { type: 'text', text: ' and its log is ' },
  {
    type: 'attachment',
    name: 'ci.log',
    id: 'a5',
    mediaType: 'text/plain',
    sizeBytes: 2048,
    sha256: 'demo-log',
    snippet: pastedSnippet,
  },
  { type: 'text', text: '. The manifest on my laptop is ' },
  {
    type: 'reference',
    reference: encodeRemoteReference('alex-mbp', '/Users/alex/Projects/shop/package.json'),
  },
  { type: 'text', text: '.' },
]
/** A message in the dialect: formatting, a link and a bare URL, and a fenced block. */
const formattedBubble: UserContentBlock[] = [
  {
    type: 'text',
    text: [
      'The **modal** `padding` is off by *one* column, ~~not two~~. See [the layout notes](docs/layout.md) and https://example.com/issues/42.',
      'Keep snake_case names and ~/.zshrc as they are; 2 * 3 stays text.',
      '```ts',
      'export const padding = 12',
      '```',
    ].join('\n'),
  },
]
/** The composer's drafts: the Markdown with a mark where each capsule stands, beside their files in that order. */
const composerDrafts = {
  ready: `The spec is ${ATTACHMENT_MARK}, the screenshot from CI is ${ATTACHMENT_MARK} and the manifest on my laptop is ${ATTACHMENT_MARK}.`,
  picture: `${ATTACHMENT_MARK} Why is this off?`,
  uploading: `Wait for ${ATTACHMENT_MARK} and ${ATTACHMENT_MARK} to arrive.`,
  attaching: `Compare ${ATTACHMENT_MARK} with ${ATTACHMENT_MARK}, pasted from another conversation.`,
  failed: `The capture ${ATTACHMENT_MARK} did not upload; the log ${ATTACHMENT_MARK} did.`,
  missing: `Compare ${ATTACHMENT_MARK} with ${ATTACHMENT_MARK}.`,
  pasted: `Summarize ${ATTACHMENT_MARK}, then check it against ${ATTACHMENT_MARK}.`,
  lines: `Before: ${ATTACHMENT_MARK}\nAfter: ${ATTACHMENT_MARK}\nThe log ${ATTACHMENT_MARK} shows the padding change landing after the layout pass, which is why the modal jumps by one column on the first frame.`,
  long: 'The login test in packages/web/src/auth.test.ts is failing after the session cookie rename, and the fixture it reads has the old name.',
  formatted: [
    'The **modal** `padding` is off by *one* column, ~~not two~~. See [the layout notes](docs/layout.md) and https://example.com/issues/42.',
    '```ts',
    'export const padding = 12',
    '```',
  ].join('\n'),
}
/**
 * A file on each of three lines, the last line long enough to wrap: no
 * capsule makes its line taller or reaches into the line under it.
 */
const lineCapsulesBubble: UserContentBlock[] = [
  { type: 'text', text: 'Before: ' },
  { type: 'image', source: { type: 'ref', ref: galleryBlobs.screenshot, mediaType: 'image/png', width: 480, height: 300 } },
  { type: 'attachment', name: 'before.png', id: 'a6', mediaType: 'image/png', sizeBytes: 48211, sha256: 'demo-before' },
  { type: 'text', text: '\nAfter: ' },
  { type: 'image', source: { type: 'ref', ref: galleryBlobs.screenshot, mediaType: 'image/png', width: 480, height: 300 } },
  { type: 'attachment', name: 'after.png', id: 'a7', mediaType: 'image/png', sizeBytes: 48011, sha256: 'demo-after' },
  { type: 'text', text: '\nThe log ' },
  { type: 'attachment', name: 'ci.log', id: 'a8', mediaType: 'text/plain', sizeBytes: 2048, sha256: 'demo-log', snippet: pastedSnippet },
  { type: 'text', text: ' shows the padding change landing after the layout pass, which is why the modal jumps by one column on the first frame.' },
]
const userBubble = [
  {
    type: 'text' as const,
    text: 'The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.',
  },
]
const queuedBubble = [
  {
    type: 'text' as const,
    text: 'Also add a case for the expired cookie.',
  },
]
const stuckBubble = [
  {
    type: 'text' as const,
    text: 'Do not touch the cookie helper. Only fix the assertion.',
  },
]
const pendingSteerShown = ref(true)
const queuedShown = ref(true)
/**
 * Recall on a waiting message's specimen, which has no composer, so it
 * acts as the product's does beside a draft: the row goes, and the toast's
 * Undo brings it back as sending it again would.
 */
function recallSpecimen(show: (shown: boolean) => void): void {
  show(false)
  showRecalled(() => show(true))
}
const functionalCollapsed = ref(false)
const functionalExpanded = ref(true)
const functionalTool = ref(false)
const functionalShellExpanded = ref(true)
const functionalShellLive = ref(true)
const functionalShellScript = ref(true)
const functionalShellBash = ref(true)
const functionalShellLongLine = ref(true)
/** Calls whose results carry media, as the transcript shows them. */
/** How many pictures a row holds, with and without a tag after its title, at the widths the Tool Media lab shows. */
const rowPictureLab = [1, 5, 20].flatMap((count) => [false, true].map((tagged) => ({
  variant: `${countOf(count, 'picture')}${tagged ? ' · with a tag' : ''}`,
  pictures: rowPictureSamples(count),
  tagged,
})))
/** Rolls the lab's faces to another icon, as a run's face does when it ends: the row's line holds the same after it. */
const rowPictureRolled = ref(false)
const rowPictureWidths = [{ name: 'wide', width: 900 }, { name: 'narrow', width: 320 }]
const toolMediaSpecimens = [
  { variant: 'shell · one screenshot', block: screenshotTool },
  { variant: 'shell · four screenshots', block: fourScreenshotsTool },
  { variant: 'shell · twelve screenshots, the rest counted', block: twelveScreenshotsTool },
  { variant: 'shell · several images', block: screenshotsTool },
  { variant: 'shell · tall image', block: fullPageTool },
  { variant: 'shell · wide image', block: wideImageTool },
  { variant: 'shell · small image', block: smallImageTool },
  { variant: 'shell · eight wide frames, counted', block: shortFramesTool },
  { variant: 'shell · video', block: recordingTool },
  { variant: 'shell · video of unknown size', block: unsizedRecordingTool },
  { variant: 'status · image', block: statusImageTool },
  { variant: 'shell · video not stored', block: notStoredVideoTool },
  { variant: 'shell · image cannot load', block: missingImageTool },
  { variant: 'shell · binary stdout, not shown', block: binaryStdoutTool },
]
const functionalShellFiles = ref(false)
const changeCaseOpen = reactive<Record<string, boolean>>({})
const functionalThinkingStartedAt = new Date().toISOString()
const functionalThinkingEndedAt = new Date(
  Date.parse(functionalThinkingStartedAt) + 8_000,
).toISOString()

/**
 * Thinking that streams a few words at a time and then ends, as a reply's
 * thinking does: the row stays folded unless the reader opens it, and stays
 * as the reader left it when the thinking ends. Replay streams it again.
 */
const streamingThinking = reactive({ text: '', startedAt: new Date().toISOString(), endedAt: null as string | null })
const streamingThinkingOpen = ref(false)
let streamingThinkingTimer: ReturnType<typeof setInterval> | undefined
function stopStreamingThinking(): void {
  clearInterval(streamingThinkingTimer)
  streamingThinkingTimer = undefined
}
function playStreamingThinking(): void {
  stopStreamingThinking()
  const words = thinkingText.split(/(?<=\s)/)
  let shown = 0
  streamingThinking.text = ''
  streamingThinking.startedAt = new Date().toISOString()
  streamingThinking.endedAt = null
  streamingThinkingTimer = setInterval(() => {
    shown += 1
    streamingThinking.text = words.slice(0, shown).join('')
    if (shown >= words.length) {
      stopStreamingThinking()
      streamingThinking.endedAt = new Date().toISOString()
    }
  }, 120)
}
// Requesting covers a recovery in flight and the agent's own retries: one wait, one word.
const activityKinds: ActivityKind[] = ['requesting', 'connecting']
// The wait the slot specimens show began when the page opened.
const activitySince = Date.now()
const compactionCases: { state: CompactionCase; variant: string }[] = [
  { state: 'running', variant: 'compacting' },
  { state: 'done', variant: 'compacted, at its trigger' },
  { state: 'failed', variant: 'failed' },
]
const compactionSteers = [{ id: 'compaction-steer', content: [{ type: 'text' as const, text: 'Also run the whole auth suite.' }] }]

/** The stand-in session an edit is sent to: it replaces the message and what follows with the edit and a reply. */
const editHost: MessageEditHost = {
  get: () => messageEdit.value,
  set: (state) => { messageEdit.value = state },
  send: async (request) => {
    const blocks = session.blocks
    const index = blocks.findIndex((block) => block.id === request.targetBlockId)
    if (index < 0) {
      throw new Error('Message not found')
    }
    session.blocks = [
      ...blocks.slice(0, index),
      {
        type: 'user', id: request.operationId, turnId: request.operationId,
        createdAt: new Date().toISOString(), model: demoModel,
        content: request.content as UserContentBlock[], preamble: null,
      },
      {
        type: 'text', id: `reply-${request.operationId}`,
        createdAt: new Date().toISOString(), model: demoModel,
        text: 'Continuing from the edited message.',
      },
    ]
    editRevision.value += 1
  },
}

function submitEdit(): Promise<void> {
  return submitMessageEdit(editHost)
}


const sessionRestore = new RestoreSweep()
const composerArchived = ref(true)
// The product clears a hold when the Cloud status it polls says the reset ended.
const composerHeld = ref(false)
let composerHoldTimer: ReturnType<typeof setTimeout> | undefined
function holdComposer(): void {
  composerHeld.value = true
  clearTimeout(composerHoldTimer)
  composerHoldTimer = setTimeout(() => { composerHeld.value = false }, 4000)
}
onBeforeUnmount(() => clearTimeout(composerHoldTimer))
// The model catalog failed to load; Retry reloads it the way the product revalidates.
const composerModelLoad = ref<'loading' | 'ready' | 'failed'>('failed')
const composerModelRestore = new RestoreSweep()
function retryModels(): void {
  composerModelRestore.start((phase) => {
    composerModelLoad.value = phase
  })
}
function breakModels(): void {
  composerModelRestore.stop()
  composerModelLoad.value = 'failed'
}
const liveLoad = ref<SessionLoad>('failed')
const liveHasTranscript = computed(() => liveLoad.value === 'ready')
const livePane = computed(() =>
  sessionPaneStatus(liveLoad.value, liveHasTranscript.value),
)
const missingKind = ref<'missing' | 'empty'>('missing')

function retrySession(): void {
  sessionRestore.start((phase) => {
    liveLoad.value = phase
  })
}

function breakSession(): void {
  sessionRestore.stop()
  liveLoad.value = 'failed'
}

onMounted(() => {
  playStream()
  playTurn('turn')
  playStreamingThinking()
})

onBeforeUnmount(() => {
  sessionRestore.stop()
  composerModelRestore.stop()
  stopStreamingThinking()
})
</script>

<template>
  <div :class="fillPane ? 'flex min-h-0 flex-1 flex-col' : 'flex flex-col gap-10'">
    <template v-if="view === 'tabs'">
      <GallerySection
        title="Motion"
        note="The work panel’s browser tabs, and every motion their strip owns, in a narrow frame that overflows and one at the pane width. Changes and Files, pinned ahead of the tabs, keep one shape whatever is selected: each its icon and its own short title, Changes with its counts after it, and Files never the open file’s name, which is in its tooltip; a divider parts them from the tabs. Tabs fit their mark and title up to 160px and up to an equal share of the strip; longer titles end in an ellipsis, so no title, a selected one included, widens a tab past its share, and selecting a tab changes no tab’s width. As in Chrome, a strip that runs out of room first narrows every tab to the same share, down to 88px, their mark and a few characters, and only then scrolls, so a tab opened in the background normally shows; once it scrolls, a button at its end lists every tab by its mark and title, as VS Code’s Show Opened Editors does, and picking one selects it and brings it into view. On a phone (the Phone page) no tab shrinks: each keeps its width, the pinned tabs too, and the whole row scrolls under a finger, with no scrollbar. A tab opens by growing from nothing and closes by collapsing. The selected tab always shows whole, its close control included, and never under a fade: selecting a tab, opening one, the agent showing one, the strip resizing and the selected tab’s title widening it each scroll, with the same timing as the tab, until it shows whole with a fade’s width beside it; where the strip has no room for that, the fades give way beside it, and no tab is wider than the strip has room for, so a very narrow strip truncates the selected tab’s title. Unselected tabs may lie under the fades, and so may the selected tab once the user scrolls it under an edge: the scrolled edge fades as the other does. Closing scrolls back when the end comes into reach. Wherever the strip comes to rest, a tab mark its edge cuts stays under the fade’s solid start, so a mark at a scrolled edge shows whole or not at all, never sliced. The crowded strip has the product’s panel width and five tabs, the last selected; the overflowing one 25, and the last specimen a long file name open in File. Play all runs through the cases; the tabs, their close controls and their menus work too. The tabs act as a web browser’s: a middle click closes one, a drag moves one along the strip while the others step aside, and Escape puts it back; a focused tab moves the focus with the arrows, Home and End, and Enter or Space selects it. A tab’s tooltip comes only when the pointer moved onto it and rests there, never after a click, nor for a tab that slid under a pointer at rest, as a new one does."
      >
        <GallerySpecimen variant="narrow · overflows" wide>
          <GalleryTabStripDrive width="32rem" />
        </GallerySpecimen>
        <GallerySpecimen variant="crowded · panel width, the last of five selected" wide>
          <GalleryTabStripDrive width="37.25rem" :pages="5" select="last" />
        </GallerySpecimen>
        <GallerySpecimen variant="overflowing · panel width, 25 tabs, the last selected" wide>
          <GalleryTabStripDrive width="37.25rem" :pages="25" select="last" />
        </GallerySpecimen>
        <GallerySpecimen variant="wide · pane width" wide>
          <GalleryTabStripDrive />
        </GallerySpecimen>
        <GallerySpecimen variant="long file name · panel width, File selected" wide>
          <GalleryTabStripDrive width="37.25rem" :pages="3" file="docs/notes/2026-10-17-crowded-work-panel-regression-investigation-notes.md" />
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'composer'">
      <GallerySection
        title="Composer"
        note="Idle through Fast Mode; text formatted as it is typed; each file a capsule where it was put, as tall as a line of its text whether the message stands on one line or the composer is open, through its upload phases, a failed upload with Retry, and a remote host file naming its device; and queue. Enter on a message that cannot go shows the send button’s reason where the pointer would. A message opens the composer as soon as it needs more than the line it is on, whether it holds lines of its own or its text outgrows the width, and closes it again when it fits; nothing is ever cut off at the line’s end. One send; while the agent works, Enter steers the running turn or queues the message, as Settings › General chooses, and ⌘⏎ outside a code block, or a ⌘-click on the send button, sends it the other way, which the button’s tooltip names; Stop stays beside it for the whole turn. Here a steer or a queued message is a toast naming what the product would do; the sessions below take them. When a later save replaced a version of the draft, a row above the input offers it by its first line: Restore exchanges it with the draft, which the row then offers, and × dismisses it. When the user’s plugins changed since the conversation opened, a row above the input offers Reload, and goes once the conversation opened again. An unavailable last model keeps the chip, warns, and blocks send. No usable model and an archived conversation both replace the input with the same snackbar: a line on the left, Configure Models or Restore Conversation on the right. A conversation its Cloud holds during a reset uses that snackbar with a spinner and no action, and the input returns with its draft when the reset ends. The model chip and the context ring stand in a row under the box, at its right edge, model first; loading and a failed load show there too. The narrow composer resizes from its corner."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen
            variant="idle · empty"
            wide
          >
            <GalleryComposer />
          </GallerySpecimen>
          <GallerySpecimen
            variant="narrow"
            wide
          >
            <!-- The frame keeps the dock's 12px around the composer, as the session does. -->
            <div class="max-w-full resize-x overflow-hidden p-3" style="width: 367px; min-width: 15.5rem">
              <GalleryComposer />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="focused"
            wide
          >
            <GalleryComposer
              focused
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="send-ready"
            wide
          >
            <GalleryComposer
              draft="The login test in packages/web/src/auth.test.ts is failing after the session cookie rename."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="a later save replaced a version · Restore exchanges them"
            wide
          >
            <GalleryComposer
              draft="Fix the login test in packages/web/src/auth.test.ts."
              replaced="Fix the login bug before the release; the session cookie was renamed."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="primary Host offline · how to start its runner again, with Copy · Move to Another Host… lists Run On’s online Hosts and moves it · on a phone a long device name wraps and the button moves under it"
            wide
          >
            <GalleryComposer
              :offline-host="{
                name: 'Alex-MacBook-Pro.local',
                start: demoDeviceStart('macos'),
                primaryHost: { id: 'old', name: 'Alex-MacBook-Pro.local', kind: 'device', state: 'offline', mark: GALLERY_MARKS.mac },
                devices: offlineMenuDevices,
              }"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="primary Host offline while its work runs · Move to Another Host… asks to stop the agent first"
            wide
          >
            <GalleryComposer
              :offline-host="{
                name: 'Old Laptop',
                start: demoDeviceStart('macos'),
                primaryHost: { id: 'old', name: 'Old Laptop', kind: 'device', state: 'offline', mark: GALLERY_MARKS.debian },
                devices: offlineMenuDevices,
              }"
              :working="{ agent: true, subagents: 0, commands: 1 }"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="primary Host offline, in a project · a device opens its directory picker first"
            wide
          >
            <GalleryComposer
              :offline-host="{
                name: 'Old Laptop',
                start: demoDeviceStart('windows'),
                primaryHost: { id: 'old', name: 'Old Laptop', kind: 'device', state: 'offline', mark: GALLERY_MARKS.debian },
                devices: offlineMenuDevices,
                chooseDirectory: true,
              }"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="plugins changed since it opened · Reload opens it with the plugins on"
            wide
          >
            <GalleryComposer
              plugins-changed
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="multiline"
            wide
          >
            <GalleryComposer
              draft="The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.&#10;Keep the fix in that file."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="longer than its line"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.long"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="formatting · as it will look"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.formatted"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · capsules in the text"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.ready"
              :attachments="[
                {
                  id: 'pdf',
                  name: 'login-failure.pdf',
                  phase: 'ready',
                },
                {
                  id: 'png',
                  name: 'login-fail.png',
                  src: demoImageUrl,
                  phase: 'ready',
                },
                {
                  id: 'ref',
                  host: 'alex-mbp',
                  path: '/Users/alex/Projects/shop/package.json',
                },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="attachments · a picture on one line">
            <GalleryComposer
              :draft="composerDrafts.picture"
              :attachments="[{ id: 'png', name: 'IMG_7325.png', src: demoImageUrl, phase: 'ready' }]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · on consecutive lines"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.lines"
              :attachments="[
                { id: 'before', name: 'before.png', src: demoImageUrl, phase: 'ready' },
                { id: 'after', name: 'after.png', src: demoImageUrl, phase: 'ready' },
                { id: 'log', name: 'ci.log', phase: 'ready' },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · uploading"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.uploading"
              :attachments="[
                {
                  id: 'up',
                  name: 'login-fail.png',
                  src: demoImageUrl,
                  phase: 'uploading',
                  progress: 0.42,
                },
                {
                  id: 'pdf-up',
                  name: 'spec.pdf',
                  phase: 'uploading',
                  progress: 0.68,
                },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · a pasted message attaching"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.attaching"
              :attachments="[
                { id: 'before', name: 'before.png', src: demoImageUrl, phase: 'attaching' },
                { id: 'after', name: 'after.png', src: demoImageUrl, phase: 'attaching' },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · an upload failed · Retry"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.failed"
              :attachments="[
                {
                  id: 'failed',
                  name: 'capture.png',
                  src: demoImageUrl,
                  phase: 'failed',
                },
                {
                  id: 'log',
                  name: 'ci.log',
                  phase: 'ready',
                  snippet: pastedSnippet,
                },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · a file no longer available · delete or attach again"
            wide
          >
            <GalleryComposer
              :draft="composerDrafts.missing"
              :attachments="[
                { id: 'gone', name: 'image.png', phase: 'missing' },
                { id: 'kept', name: 'after.png', src: demoImageUrl, phase: 'ready' },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="drop · where the files land"
            wide
          >
            <GalleryComposer
              draft="Drop a file between these words."
              dropping
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="paste · a long text becomes pasted-text.txt"
            wide
          >
            <GalleryComposer
              placeholder="Paste 2000+ characters or 40+ lines here…"
              :draft="composerDrafts.pasted"
              :attachments="[
                {
                  name: 'pasted-text.txt',
                  phase: 'ready',
                  snippet: pastedSnippet,
                },
                { name: 'release-notes.md', phase: 'ready', snippet: markdownSnippet },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attach menu"
            wide
          >
            <GalleryOverlayWell>
              <GalleryComposer
                attach-open
              />
            </GalleryOverlayWell>
          </GallerySpecimen>
          <GallerySpecimen
            variant="running · Enter steers or queues as the setting says, ⌘⏎ the other way · Stop stays beside it"
            wide
          >
            <GalleryComposer
              running
              draft="Also add a case for the expired cookie."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="stop"
            wide
          >
            <GalleryComposer
              running
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="fast"
            wide
          >
            <GalleryComposer
              service-tier-id="priority"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="no reasoning"
            wide
          >
            <GalleryComposer
              selected-provider-id="openai"
              selected-model-id="gpt-5"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="context · warning"
            wide
          >
            <GalleryComposer
              :usage="usageAt(0.75)"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="model unavailable"
            wide
          >
            <GalleryComposer
              draft="The login test in packages/web/src/auth.test.ts is failing after the session cookie rename."
              :providers="openaiOnlyProviders"
              selected-provider-id="anthropic"
              selected-model-id="claude-sonnet"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="no models"
            wide
          >
            <GalleryComposer
              :providers="offlineProviders"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="models failed · live"
            wide
          >
            <div class="mb-2">
              <Button
                variant="ghost"
                size="sm"
                :disabled="composerModelLoad === 'failed'"
                @click="breakModels"
              >Break</Button>
            </div>
            <GalleryComposer
              :model-load="composerModelLoad"
              @retry-models="retryModels"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="held · Cloud is resetting"
            wide
          >
            <GalleryComposer
              hold="Cloud is resetting."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="held · live · the draft returns when the reset ends"
            wide
          >
            <div class="mb-2">
              <Button
                variant="ghost"
                size="sm"
                :disabled="composerHeld"
                @click="holdComposer"
                >Reset Cloud</Button
              >
            </div>
            <GalleryComposer
              draft="Run the login test again."
              :hold="composerHeld ? 'Cloud is resetting.' : null"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="archived"
            wide
          >
            <GalleryComposer
              archived
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="archived · live"
            wide
          >
            <div class="mb-2">
              <Button
                variant="ghost"
                size="sm"
                :disabled="composerArchived"
                @click="composerArchived = true"
                >Archive</Button
              >
            </div>
            <GalleryComposer
              :archived="composerArchived"
              @restore="composerArchived = false"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="Saved for Later"
        note="A message saved for later is one the user keeps to send later, and nothing sends it but the user. It is not the conversation’s draft, the message the composer holds, which it leaves to be saved. While the message holds text or files, Save for Later (⌘S) stands left of Stop and Send; it keeps the whole message, its files included, and empties the composer. The entry under the composer, at the left of the model’s row, reads 1 Saved or 3 Saved and is absent while there is none. Pressed, it opens the list under the composer, which rises to make room: about four rows tall, the fifth half in view, scrolling within. A row shows the message’s first line and, when it has files, their count, and no time; pressing it edits the message. Edit, Send and Delete always stand at its end, after the count, on every width: quiet, and in the normal icon color while the pointer or the focus is on the row. In a narrow composer the entry keeps its mark and the count alone, with the whole label as its tip. Edit puts the message in the composer, takes it off the list and closes the list; a composer holding a message first saves that one for later, so nothing is lost. Send sends it as the composer would, steering or queueing while the agent works, and here a toast names what the product would do. Send and Delete keep the list open. Delete removes it at once, with Undo in a toast. A save the server refuses leaves its row as Not Saved in secondary red, with Retry in place of Send; nothing the user wrote is lost. Here the saved messages are the specimen’s own state; the product keeps them on the server, per conversation."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen variant="none saved · type, then Save for Later or ⌘S" wide>
            <GalleryComposer
              draft="Check the server’s DNS next."
            />
          </GallerySpecimen>
          <GallerySpecimen variant="one saved · closed" wide>
            <GalleryComposer
              :saved="twoSaved.slice(0, 1)"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="two saved · closed, with a message to save" wide>
            <GalleryComposer
              draft="Check the server’s DNS next."
              :saved="twoSaved"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="seven saved · open, the composer risen" wide>
            <div class="flex min-h-[19rem] flex-col justify-end p-3">
              <GalleryComposer
                :saved="sevenSaved"
                saved-open
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="open · a message the server did not save, with Retry and Delete" wide>
            <div class="flex min-h-[13rem] flex-col justify-end p-3">
              <GalleryComposer
                :saved="notSavedOnTop"
                saved-open
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="running · Stop beside Save for Later and Send" wide>
            <GalleryComposer
              running
              draft="Then check the IPv6 records too."
              :saved="twoSaved"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="narrow · open" wide>
            <div class="flex min-h-[19rem] max-w-full resize-x flex-col justify-end overflow-hidden p-3" style="width: 367px; min-width: 15.5rem">
              <GalleryComposer
                draft="Check the server’s DNS next."
                :saved="sevenSaved"
                saved-open
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="Workflow Menu"
        note="The composer’s lower left has the Workflow menu, which shows the conversation’s choice. Global Workflow follows Settings › Subagents; No Subagents turns subagents off for this conversation; each workflow the project keeps in .demi/workflows/ is an option of its own, by its name. A file that does not read, or that is a symbolic link, stands in the menu with why, as its tip, and cannot be chosen; a .demi/workflows directory that is a link makes the menu say so in one row. A new conversation offers the menu before its first message and keeps the choice it was given. It starts with the model and the workflow last chosen in its project, or outside projects; a project never used starts as a conversation outside one does, and a remembered workflow the project no longer has starts as No Subagents. A chosen workflow that is gone shows a warning beside its id, and the menu lists it with why until another is chosen. The choice shows at once, and applies from the next spawn. In a narrow composer the menu keeps its mark alone, with the choice as its tip. Here choosing changes the specimen’s own choice; the product keeps it with the conversation, and lists the workflows from the conversation’s file watch."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen variant="a new conversation starts with its project’s choices" wide>
            <GalleryProjectStart />
          </GallerySpecimen>
          <GallerySpecimen variant="Global Workflow · a project with workflows" wide>
            <GalleryComposer :workflows="projectWorkflows" />
          </GallerySpecimen>
          <GallerySpecimen variant="Review Team chosen, with one saved message" wide>
            <GalleryComposer
              :workflows="projectWorkflows"
              :workflow-choice="{ type: 'workflow', id: 'review' }"
              :saved="twoSaved.slice(0, 1)"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="the chosen workflow is gone" wide>
            <GalleryComposer
              :workflows="projectWorkflows"
              :workflow-choice="{ type: 'workflow', id: 'explore' }"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="no workflows · No Subagents chosen" wide>
            <GalleryComposer
              :workflows="noWorkflows"
              :workflow-choice="{ type: 'none' }"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="the workflows directory is a link" wide>
            <GalleryComposer
              :workflows="{ state: 'unavailable', reason: 'Workflows are files of the project, not links.' }"
              :workflow-choice="{ type: 'workflow', id: 'review' }"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="reading the workflows" wide>
            <GalleryComposer :workflows="{ state: 'reading' }" />
          </GallerySpecimen>
          <GallerySpecimen variant="narrow" wide>
            <div class="flex max-w-full resize-x flex-col justify-end overflow-hidden p-3" style="width: 367px; min-width: 15.5rem">
              <GalleryComposer
                :workflows="projectWorkflows"
                :workflow-choice="{ type: 'workflow', id: 'review' }"
                :saved="twoSaved"
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="RequestCard"
        note="An agent’s command needs the user’s answer: the oldest request waits above the composer, below the transcript and over the dock’s chips, while the transcript and the composer stay usable. It says what the conversation would be allowed, the command the agent ran, the subagent that ran it, what a grant allows, and which one of how many it is. A command that brings a host the conversation may not reach asks to reach it, by the host’s name, or, for a pairing, to pair a computer and reach it; no command needs a category and a host at once. Allow for This Conversation grants the request’s category and decides every request of it; a request with a host says Allow, since a host is allowed for this conversation by nature, and adds the host to those the conversation may reach; Deny decides this one, and the next takes its place. A category the user’s command set no longer declares shows its id. A credential request (Save a Credential) shows the credential’s name, the agent’s reason and one input per field under its label, a hidden one with an eye that shows what was typed, and how long it is kept (Kept for 1 hour, Kept until you remove it), which the agent decided, so the card only says it; Save waits until every field has a value, and Decline answers the agent that the user declined. A renewal of a saved credential says Update the Credential and asks for its fields empty. A refusal of the values shows under the form, which keeps what was typed. An allowance request asks to let the conversation use a stored credential; one for a credential the user removed says Removed and offers only Don’t Allow."
      >
        <div class="specimen-stack">
          <GallerySpecimen
            v-for="specimen in permissionSpecimens"
            :key="specimen.variant"
            :variant="specimen.variant"
            wide
          >
            <div class="w-full max-w-[44rem]">
              <RequestCard
                v-if="specimen.requests.length"
                :requests="specimen.requests"
                :error="specimen.error"
                @decide="(id, decision) => { specimen.error = null; specimen.requests = decidePermission(specimen.requests, id, decision) }"
              />
              <div v-else class="flex items-center gap-3 text-chrome text-fg-subtle">
                Every request is decided.
                <Button size="sm" @click="specimen.requests = specimen.fixture(); specimen.error = specimen.refusal">Show Again</Button>
              </div>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="SessionDock"
        note="What waits over the composer stacks one 8px step apart: the cards, then the chips, then the composer. The chips sit directly on the composer and nothing comes between them: a permission card, the offline Host’s card or a notice stands above the chips. A part that is not there takes no room, so without chips a card sits one step above the composer, as the chips do without a card. The permission card stands over the dock’s top rather than in it: the transcript keeps clear of it, and a panel the dock opened keeps its place under it (Panel Under a Permission Card). The scroll-to-bottom control floats over the transcript at the dock’s top right and takes no room either."
      >
        <div class="specimen-stack">
          <GallerySpecimen
            v-for="specimen in dockSpecimens"
            :key="specimen.variant"
            :variant="specimen.variant"
            wide
          >
            <div class="flex w-full max-w-[44rem] flex-col gap-3">
              <div
                v-if="specimen.asks && !specimen.requests.length"
                class="flex items-center gap-3 text-chrome text-fg-subtle"
              >
                Every request is decided.
                <Button size="sm" @click="specimen.requests = [rootRequest()]">Show Again</Button>
              </div>
              <!-- The primary Host comes back: its card goes and Resume may go, as in the product. -->
              <div v-if="specimen.device">
                <Button size="sm" variant="ghost" @click="specimen.offline = !specimen.offline">
                  {{ specimen.offline ? 'Connect Runner' : 'Disconnect Runner' }}
                </Button>
              </div>
              <!-- The permission card stands over the dock's top and takes no room in it: the frame leaves
                   it room above, over the offline card too where one shows. -->
              <div
                class="flex flex-col justify-end rounded-lg bg-surface p-3"
                :class="specimen.asks ? specimen.offline ? 'min-h-[30rem]' : 'min-h-[24rem]' : ''"
              >
                <SessionDock>
                  <template v-if="specimen.requests.length" #above>
                    <RequestCard
                      :requests="specimen.requests"
                      @decide="(id, decision) => (specimen.requests = decidePermission(specimen.requests, id, decision))"
                    />
                  </template>
                  <template #chips>
                    <template v-if="specimen.chips">
                      <Tooltip
                        :content="specimen.offline ? `${specimen.host} is still offline` : undefined"
                        :disabled="!specimen.offline"
                        class="inline-flex"
                      >
                        <SessionDockChip :disabled="specimen.offline" @click="productWould('Resume the Turn')">
                          <Play :size="ICON_PX.in28" />
                          {{ agentText('dock.resume') }}
                        </SessionDockChip>
                      </Tooltip>
                      <AgentsChip :agents="agents" @open="productWould('Open the Agents Window')" />
                    </template>
                  </template>
                  <!-- The composer's cards, such as the offline Host's, stand above the chips. -->
                  <GalleryComposer
                    :offline-host="specimen.offline ? dockOfflineHost(specimen.host) : null"
                    @moved="(_host, name) => { specimen.host = name; specimen.offline = false }"
                  />
                </SessionDock>
              </div>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="ModelSelector"
        note="The model dropdown aligns to the trigger’s right edge, extending to the left. A new conversation inherits the saved model, reasoning and Fast Mode; the product adapter persists this preference to the backend. A model over 500K offers a Context row: the limit is the user’s for that model in every conversation, so the context specimens share one store, and the usage indicator counts against the limit."
      >
        <div class="specimen-stack">
          <GallerySpecimen variant="new conversation · saved choice">
            <GalleryModelPreference />
          </GallerySpecimen>
          <GallerySpecimen variant="chip">
            <ModelSelector
              :providers="demoProviders"
              :models="composerModels"
              :settings="selectorSettings"
              @change="changeSelector"
              @context-limit="setGalleryContextLimit"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="fast">
            <ModelSelector
              :providers="demoProviders"
              :models="composerModels"
              :settings="fastSettings"
              @change="changeFast"
              @context-limit="setGalleryContextLimit"
            />
          </GallerySpecimen>
          <GalleryOverlayWell size="wide">
            <GallerySpecimen variant="menu">
              <ModelMenu
                :providers="demoProviders"
                :models="composerModels"
                :settings="selectorSettings"
                @change="changeSelector"
                @context-limit="setGalleryContextLimit"
              />
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="wide">
            <GallerySpecimen variant="context · 200K model, no row">
              <GalleryContextLimit provider-id="anthropic" model-id="claude-haiku" />
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="wide">
            <GallerySpecimen variant="context · 800K model: full, 300K, 200K">
              <GalleryContextLimit provider-id="openai" model-id="gpt-long" />
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GalleryOverlayWell size="wide">
            <GallerySpecimen variant="context · 1M model: full, 500K, 300K, 200K">
              <GalleryContextLimit provider-id="anthropic" model-id="claude-opus" />
            </GallerySpecimen>
          </GalleryOverlayWell>
          <GallerySpecimen variant="context · limited to 300K, with its usage">
            <GalleryContextLimit provider-id="google" model-id="gemini-pro" :used-tokens="150000" />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="SessionDockChip"
        note="28px capsule with status dots. The Agents chip counts the live children and Running the commands, each dot breathing gently as every running thing does (StatusDot), and shows only while one runs, as Running shows only while a command runs; a finished child opens from its receipt's name or a link to it. Running and Agents open their windows; a second click closes them (Agents Chip in Windows)."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="resume">
            <SessionDockChip @click="productWould('Resume the Turn')">
              <Play :size="ICON_PX.in28" />
              {{ agentText('dock.resume') }}
            </SessionDockChip>
          </GallerySpecimen>
          <GallerySpecimen variant="running">
            <TerminalChip :terminals="terminals" @open="productWould('Open the Terminals Window')" />
          </GallerySpecimen>
          <GallerySpecimen variant="ready">
            <SessionDockChip dot="success" @click="productWould('Show What Is Ready')">Ready</SessionDockChip>
          </GallerySpecimen>
          <GallerySpecimen variant="idle">
            <SessionDockChip dot="muted" @click="productWould('Show What Is Idle')">Idle</SessionDockChip>
          </GallerySpecimen>
          <GallerySpecimen variant="agents · running">
            <AgentsChip :agents="agents" @open="productWould('Open the Agents Window')" />
          </GallerySpecimen>
          <GallerySpecimen variant="agents · none running · hidden">
            <AgentsChip :agents="finishedOnly" @open="productWould('Open the Agents Window')" />
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'blocks'">
      <GallerySection
        title="Assistant Message"
        note="Intermediate updates have no copy, fork or timestamp toolbar. Final replies keep their toolbar, including while a later user request runs. Preview fork success, pending creation, and a retry after failure. Share, beside Fork, opens the share dialog at once and makes the answer’s link in it, copied once it stands (Dialogs); it waits, like Fork, until the message completes, and on an answer that has a share it opens the dialog on that link. The conversation header’s ⋯ menu holds Share Conversation… in the same way."
      >
        <GallerySpecimen variant="message footer" wide>
          <GalleryAssistantMessages />
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="Attachments in a Message"
        note="What the agent gave the user with demi attachment upload, delivered as a block of its own: <attachment id=&quot;a3&quot; /> on a line of its own. An image, and a video’s first frame with its play mark, is a tile: scaled down with its proportions kept to fit a third of the message’s width less the two 8px gaps, and 240px tall, never enlarged and never cropped, so three fill a line; the tall page and the wide timeline each show whole. Its box comes from its record’s pixel size before its bytes arrive and follows the message’s width; the demo’s record carries no size, so its frame is 16:9 until it arrives. Every image has a 1px edge in the theme’s faint line over its outermost pixels and the same small corner as the tile, so a screenshot on white or on dark does not melt into the page. A lone one stands on a line of its own 12px from the text above and below; images and videos whose blocks follow each other stand side by side 8px apart and wrap. Resting the pointer on any of them for 300 ms shows it large in a card above it, with its name and pixel size under it; a phone shows none. A click shows an image large and plays a video in the viewer. Audio is its inline player. Any other file is a card, 360px wide at most, with the file tree’s icon, its name, and its kind and size: a click opens it in the work panel, previewed as the File view previews a file from the attachment’s copy, and only Download downloads; cards on consecutive lines stack. The tag in a sentence, and a link to attachment:a3, are text, and a number the conversation does not have, or one the reply’s agent did not upload, such as the user’s file or another agent’s, says so in the block’s place. In a list, a screenshot’s block on the line after an item’s words stands on its own line under them, and two stand side by side under them."
      >
        <GallerySpecimen variant="a lone image" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="loneImageMarkdown" />
        </GallerySpecimen>
        <GallerySpecimen variant="images with a white and a dark background" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="edgedImagesMarkdown" />
        </GallerySpecimen>
        <GallerySpecimen variant="a run of two" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="twoMediaMarkdown" />
        </GallerySpecimen>
        <GallerySpecimen variant="a run of five, wrapping" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="fiveMediaMarkdown" />
        </GallerySpecimen>
        <GallerySpecimen variant="a voice-over, played where it is" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="audioAttachmentMarkdown" />
        </GallerySpecimen>
        <GallerySpecimen variant="files as cards, the tag in a sentence, a missing number, a recording of unknown size" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="attachmentMarkdown" />
        </GallerySpecimen>
        <GallerySpecimen variant="after a list item’s words" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="listAttachmentMarkdown" />
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="Files in a Message"
        note="Every link in a message wears an icon, so each reads as the kind of thing it opens. A file of the Host a reply names is a link with the file's icon before its name, the icon the file tree shows, and 0.2em of space on each side, outside its underline, as an agent's link has; a commit link, [text](commit:3f2a9c1), or [text](commit:vendor/parser@3f2a9c1) for a commit of the repository in that folder, read as a relative link's path is, shows the commit icon the branch menu uses for a detached head before its text, spaced the same way, and a click opens the commit in the Change view of the frame's panel; a web page's link shows the link icon, spaced the same way, and opens in a new tab; a link whose text is an image, as a badge's is, wears none. A click on a file opens it in the File view of the frame's panel, and a #L42 after the path opens it at line 42, scrolled there and marked for a moment as a search result is. A link's color says where it leads: a web page's link is in the accent color, as a link on a page is, while a file's and a commit's read as part of the sentence, in the prose's color over a faint underline, as an agent's does, told apart by their icons, the commit icon in orange as Git's own mark; fixed colors for each kind would meet the accent the user picks (Settings › Appearance), an orange accent making the web's links read as commits. A link is one color: code in its text drops code's look and reads as the link's words, in the link's color, while code outside a link keeps its own; a Markdown document's links wear no icon and keep code's monospace and fill in the link's color (Markdown, Edge Cases, Shown As Document). Inline code that reads as a path, one with a slash or a name with an extension and an optional :line, opens too when the Host has the file, as a link of words with the file's icon: the page asks the Host once for the reply's paths together, after the reply is complete. Here the gallery answers for the Host from its demo workspace, so src/auth/token.ts, which it lacks, stays code; a bare name such as cookie is never asked about."
      >
        <GallerySpecimen variant="links, a line, code in links, code paths the Host has and lacks" wide>
          <GalleryMessageFiles :files="sessionFiles" :cwd="workspace.root" :content="namedFilesMarkdown" />
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="Agents in a Message"
        note="A reply links a subagent as agent:4, the number spawn printed: its mark, the robot with the dot of its state as its tab in the Agents window wears it, then the text. It is drawn in the text's own colors, not a web link's accent, as a mention in Slack or Linear reads as part of the sentence: the text in the prose's color at medium weight over a dotted underline in the faintest text color, 1px, 3px below the text, and the mark in the secondary text color. It starts with an icon, so, as a file's link, it keeps 0.2em of space on each side, inside its hover fill and outside its underline: in the Chinese sentence the character before the mark and the colon after the name do not touch it. Punctuation of the Latin kind straight after the name, such as the semicolon after main agent, stays against it and keeps no space: the 0.2em after the name is dropped there. It flows as words do: a long name wraps at the line's end like any text, its first line starting where the text before it ends, the mark staying with the first word, and the hover fill and the underline following each line it spans, in a narrow column and a table's cell alike. It is reached with Tab and opened with Return or Space, as a button is. Pointed at, it shows the hover fill behind both and no tip: the link’s text already names the agent. A click opens the Agents window on that agent's tab, a finished one as Completed picks it, and a click when the window shows that tab leaves it as it is; stopping an agent there takes the dot off its mark, as a stopped conversation's row is settled. agent:0, the main agent, brings the main transcript into view. A number the conversation does not have is its text alone."
      >
        <GallerySpecimen variant="running, finished, failed, stopped, main, unknown" wide>
          <GalleryAgentLinks />
        </GallerySpecimen>
        <GallerySpecimen variant="long names in a narrow column and a table's cell" wide>
          <GalleryAgentLinks :markdown="agentLinksWrapMarkdown" narrow />
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="UserBlock"
        note="User; files as capsules where they were put: pointed at, an image or a video shows large in its preview card after 300 ms, with the file’s name and pixel size under it, and a text file its opening lines; formatting; pending steer, queued, and stuck. Long messages have a view of their own: Lengths."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen
            variant="user"
            wide
          >
            <div class="gallery-frame gallery-user-frame bg-surface">
              <UserBlock :content="userBubble" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="user · actions"
            wide
          >
            <div
              class="gallery-frame gallery-user-frame gallery-user-frame-actions bg-surface"
            >
              <UserBlock
                :content="userBubble"
                actions-pinned
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="files in the text"
            wide
          >
            <div class="gallery-frame gallery-user-frame bg-surface">
              <UserBlock :content="attachmentBubble" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="files on consecutive lines"
            wide
          >
            <div class="gallery-frame gallery-user-frame bg-surface">
              <UserBlock :content="lineCapsulesBubble" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="formatting"
            wide
          >
            <div class="gallery-frame gallery-user-frame bg-surface">
              <UserBlock :content="formattedBubble" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="pending steer"
            wide
          >
            <div
              class="gallery-frame gallery-user-frame gallery-user-frame-actions bg-surface"
            >
              <UserBlock
                v-if="pendingSteerShown"
                :content="steerPrompt"
                pending="steer"
                @recall="recallSpecimen((shown) => (pendingSteerShown = shown))"
                @send-now="pendingSteerShown = false"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="queued"
            wide
          >
            <div
              class="gallery-frame gallery-user-frame gallery-user-frame-actions bg-surface"
            >
              <UserBlock
                v-if="queuedShown"
                :content="queuedBubble"
                pending="queued"
                @recall="recallSpecimen((shown) => (queuedShown = shown))"
                @send-now="queuedShown = false"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="stuck"
            wide
          >
            <div class="gallery-frame gallery-user-frame bg-surface">
              <UserBlock
                :content="stuckBubble"
                force-stuck
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="RunEndTag"
        note="How a row’s work went wrong, after its title: red Failed for a command that exited with an error, a tool call that failed or an agent that failed; grey Stopped, Stopping… while a stop waits for the runner, and Lost. A command’s Failed says its exit code in a tooltip, its Lost says why. The tag never shrinks, so a long title gives up its end to it."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="failed"><RunEndTag kind="failed" /></GallerySpecimen>
          <GallerySpecimen variant="failed · exit code"><RunEndTag kind="failed" hint="Exit code 2" /></GallerySpecimen>
          <GallerySpecimen variant="stopped"><RunEndTag kind="stopped" /></GallerySpecimen>
          <GallerySpecimen variant="stopping"><RunEndTag kind="stopping" /></GallerySpecimen>
          <GallerySpecimen variant="lost · why"><RunEndTag kind="lost" hint="The runner went away" /></GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection title="AgentReceiptBlock" note="Agent updates and completion receipts read as a command’s notice: what happened, then the agent by its description, underlined with dots, with Failed or Stopped after it when it went wrong. The name has no tip. A click on the agent’s name opens the Agents window over the specimens on that agent’s tab, a finished one as Completed picks it, and marks the block the receipt came from: the call that sent the update, the finished agent’s last answer, the failed agent’s failure record; the stopped agent gave none, so its tab opens at its latest. Close (×) closes the window. In a child’s transcript, a message from the main agent reads Message from main agent, main agent the name, and it brings the call that sent it into view in the main transcript. A notice row does not open, as no notice row does: no chevron, no body; what it would show is where the agent’s name leads, so the name is its one control, reached with Tab. On a narrow row the name gives way first, down to a few characters, then the words. The user’s decisions on permission requests, a move the user told the agent of, and Demi’s notice that a move the agent asked for failed name no agent and say all there is in their words, the failed move why: Move failed: the project no longer exists. These rows have no human message controls.">
        <div class="gallery-frame gallery-block-frame bg-surface">
          <GalleryReceipts :agents="agents" @abort="sessionFlow.abortSubagents" @abort-agent="sessionFlow.abortSubagent">
            <div class="specimen-stack [--agent-pad-x:0px]">
              <GallerySpecimen
                v-for="message in [...agentReceiptMessages, mainAgentMessage, ...permissionReceiptMessages, movedReceiptMessage, ...organizeReceiptMessages]"
                :key="message.id"
                :variant="receiptVariant(message)"
                wide
              >
                <AgentReceiptBlock :message="message" />
              </GallerySpecimen>
              <GallerySpecimen variant="update, failed and the main agent’s message · narrow" wide>
                <div class="flex w-[300px] flex-col">
                  <AgentReceiptBlock v-for="message in [agentReceiptMessages[0]!, agentReceiptMessages[2]!, mainAgentMessage]" :key="message.id" :message="message" />
                </div>
              </GallerySpecimen>
            </div>
          </GalleryReceipts>
        </div>
      </GallerySection>

      <GallerySection
        title="FunctionalBlock"
        note="Thinking, shell (collapsed and expanded), loading, and error. Thinking that streams is one folded row, Thinking with how long it has lasted; the reader opens it to read along, and it never opens or folds by itself, also when it ends (Replay). A shell call shows its command and output in one box: the command stays at the top, highlighted as Bash in the colors all code shows in after a muted $, and only the output under it scrolls once it fills the box. The command wraps at its spaces, as an editor’s word wrap does: a word moves to the next line whole, never split at a hyphen or inside, and only a word longer than the line breaks, at any character, as the long line’s signed address shows. The output wraps as a browser’s text does: at its spaces, a word moving to the next line whole, and only a word longer than the line (a hash, an address) breaks inside, at any character; tabs and alignment stay as the command printed them. A command longer than two lines shows two, the second ending in an ellipsis: a click on it shows it whole and another click clamps it again; one that fits is no control, and a click selects it for copying. A whole command taller than half the box scrolls on its own. A shell row shimmers while its command runs, also after its call returned with the command running on as one of the conversation’s running commands, until the command’s end arrives, and then marks how it ended as any shell row does: Stopped, or Failed for an exit code other than 0 (End Command). A command that has run more than 3 seconds wears how long, counting up, as a grey tag where Failed or Stopped would stand: Run Again starts it anew, and the tag shows from 3 seconds on, so a quick command never flashes one; the tag goes when the command ends. A stop puts Stopping… in the tag’s place until the command’s runner says that it ended, and the command counts as running meanwhile; opened, the line above the output says Stopping…. The time counts from the command’s start as the server reports it, so a reload shows the true time. Opened, the line above the output says how long it has run, Running for 1m 28s, then Exited with 0 after 2m 13s, Failed with exit code 1 after 47s or Stopped after 5s; a command that ended within a second says no time, Exited with 0. While a shell call runs, its command’s output shows under it as it comes, a line every frame here, and the box follows the newest line; scroll up to read, and it stays where you are until you scroll back to the end. Once the call returned, the call keeps the view its result stored."
      >
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack [--agent-pad-x:0px]">
            <GallerySpecimen
              variant="collapsed"
              wide
            >
              <ThinkingBlock
                v-model:open="functionalCollapsed"
                :thinking="thinkingText"
                :is-streaming="false"
                :created-at="functionalThinkingStartedAt"
                :ended-at="functionalThinkingEndedAt"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="expanded"
              wide
            >
              <ThinkingBlock
                v-model:open="functionalExpanded"
                :thinking="thinkingText"
                :is-streaming="false"
                :created-at="functionalThinkingStartedAt"
                :ended-at="functionalThinkingEndedAt"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="under one second"
              wide
            >
              <ThinkingBlock
                thinking=""
                :is-streaming="false"
                :created-at="functionalThinkingStartedAt"
                :ended-at="functionalThinkingStartedAt"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="loading"
              wide
            >
              <ThinkingBlock
                thinking=""
                :is-streaming="true"
                :created-at="functionalThinkingStartedAt"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="streaming · folded until opened"
              wide
            >
              <ThinkingBlock
                v-model:open="streamingThinkingOpen"
                :thinking="streamingThinking.text"
                :is-streaming="streamingThinking.endedAt === null"
                :created-at="streamingThinking.startedAt"
                :ended-at="streamingThinking.endedAt"
              />
              <Button size="sm" variant="ghost" class="mt-1" @click="playStreamingThinking">Replay</Button>
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · collapsed"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalTool"
                :block="shellTool"
                :input="parseToolInput(shellTool.input)"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · expanded"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalShellExpanded"
                :block="shellTool"
                :input="parseToolInput(shellTool.input)"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · live output"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalShellLive"
                :block="runningShellTool"
                :input="parseToolInput(runningShellTool.input)"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · returned, its command still runs · stopped"
              wide
            >
              <ToolShellBlock
                :block="returnedShellTool"
                :input="parseToolInput(returnedShellTool.input)"
              />
              <Button size="sm" variant="ghost" class="mt-1" :disabled="returnedCommand?.stopping" @click="toggleReturnedCommand(returnedCommand, { phase: 'aborted' })">
                {{ returnedCommand?.phase === 'running' ? 'End Command' : 'Run Again' }}
              </Button>
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · returned, its command still runs · exits 1"
              wide
            >
              <ToolShellBlock
                :block="returnedFailingShellTool"
                :input="parseToolInput(returnedFailingShellTool.input)"
              />
              <Button size="sm" variant="ghost" class="mt-1" @click="toggleReturnedCommand(returnedFailingCommand, { phase: 'exited', exitCode: 1 })">
                {{ returnedFailingCommand.phase === 'running' ? 'End Command' : 'Run Again' }}
              </Button>
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · long command"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalShellScript"
                :block="longScriptShellTool"
                :input="parseToolInput(longScriptShellTool.input)"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · a script, highlighted as Bash"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalShellBash"
                :block="bashScriptShellTool"
                :input="parseToolInput(bashScriptShellTool.input)"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · long line"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalShellLongLine"
                :block="longLineShellTool"
                :input="parseToolInput(longLineShellTool.input)"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · changed files"
              wide
            >
              <GalleryTranscript :blocks="standaloneRequest(editingShellTool)">
                <ToolShellBlock
                  v-model:open="functionalShellFiles"
                  :block="editingShellTool"
                  :input="parseToolInput(editingShellTool.input)"
                />
              </GalleryTranscript>
            </GallerySpecimen>
            <GallerySpecimen
              variant="error"
              wide
            >
              <ErrorBlock
                message="Provider aborted after 3 retries."
                code="rate_limit"
                :diagnostics="{ source: 'http', httpStatus: 429 }"
              />
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="One Row per Step"
        note="A step looks the same wherever it stands: alone, as the row of a run of that step only, which is also the row a step that still runs has under its run, and inside a run the reader opened. One component draws each kind in every place: the same icon, title, tags, pictures, chevron and shimmer, and each opens to the step alone; only the indent of an opened run differs. Each row opens and folds."
      >
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack">
            <GallerySpecimen v-for="place in stepPlaceCases" :key="place.variant" :variant="place.variant" wide>
              <GalleryStepPlaces :step="place.step" :before="place.before" />
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="Command Ends"
        note="Every shell row, a run or a look, marks a command that went wrong with a tag after its title: red Failed, with its exit code in the tooltip, grey Stopped, or grey Lost, with why in the tooltip, the same tag its notice carries; one that succeeded shows nothing, and one that runs shimmers. An open row says above its output how long its command ran and how it ended: Exited with 0 after 2m 13s, Failed with exit code 1 after 47s, Stopped after 5s; a lost one says why. A look is a shell call whose script runs demi shell status; its tag is its own command’s. A call that ran nothing, refused by the repeat guard or for its input, is red Failed and shows why once opened. A title too long for its row is cut at the end; the tag stays. Calls of tools the runtime no longer has, such as shell_status and yield in a conversation from before they were removed, are generic tool cards."
      >
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack [--agent-pad-x:0px]">
            <GallerySpecimen variant="ends" wide>
              <div class="flex flex-col">
                <ToolCallBlock v-for="block in endBlocks" :key="block.id" :block="block" />
              </div>
            </GallerySpecimen>
            <GallerySpecimen variant="ends · narrow" wide>
              <div class="flex w-[300px] flex-col">
                <ToolCallBlock v-for="block in endBlocks" :key="block.id" :block="block" />
              </div>
            </GallerySpecimen>
            <GallerySpecimen variant="calls of removed tools" wide>
              <div class="flex flex-col">
                <ToolCallBlock v-for="block in removedBlocks" :key="block.id" :block="block" />
              </div>
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="Command Reports"
        note="A command a call left running tells the agent of its progress and its end, and the transcript shows each report as a row where it arrived, so a reply the agent writes after it never appears without its cause. A report of progress has no row: the answer it wakes says what the model learned. An end shows as a notice with a bell that says what happened, in the user’s words, then names the call by its title, since a title is an imperative and never the subject of a sentence; a failure, a stop and a loss carry the tag a shell row carries, the exit code or the reason in its tooltip. Ends that arrived together are one row each. The title, underlined with dots, brings the call that started the command into view."
      >
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack">
            <GallerySpecimen v-for="specimen in reportCases" :key="specimen.variant" :variant="specimen.variant" wide>
              <GalleryCommandReports :reports="specimen.reports" />
            </GallerySpecimen>
            <GallerySpecimen variant="several in one block" wide>
              <GalleryCommandReports :reports="reportsTogether" />
            </GallerySpecimen>
            <GallerySpecimen variant="several in one block · narrow" wide>
              <div class="w-[300px]">
                <GalleryCommandReports :reports="reportsTogether" />
              </div>
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="Tool Media"
        note="The images and videos a call’s result carries stand small at the end of the call’s own line, right-aligned, at least 8px after its face’s last part, the chevron or a tag, and 4px apart, folded or open, in the order of the result: each 20px tall and 16 to 48px wide by its proportions, cropped as a thumbnail is (the tall page keeps its top, the wide timeline its middle), never enlarged, with 4px corners. The line never wraps: as many show as fit in the width the title leaves, the title truncating only down to half the row, and the rest are counted after the last one shown, +5, in the row’s faint text, which is not a control; the narrow specimen shows the title giving way to half the row, then the pictures. A command’s notice and a folded run show theirs the same way (Work Groups). Each reference carries its pixel size, so a box is final before its bytes arrive and nothing moves when they load; a video of unknown size is 16:9. A video shows its first frame with a play mark; it never plays in place. Resting the pointer on a picture for 300 ms shows it large in a card above it, with the call’s title and its pixel size under it; moving to the next picture shows that one’s at once, and leaving closes it. A click opens the medium in the viewer and closes the card. A medium that is gone shows one line under the row: that it was not stored, with the store’s reason. A medium the page cannot show is a small box that says so when pointed at. The rows with 1, 5 and 20 pictures, with and without a tag, at a wide and a narrow width, show the line holding the same after the face rolls, as a run’s face does when it ends: Roll the Faces changes their icons, and the title still truncates before a tag or the chevron is cut."
      >
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack [--agent-pad-x:0px]">
            <GallerySpecimen
              v-for="specimen in toolMediaSpecimens"
              :key="specimen.variant"
              :variant="specimen.variant"
              wide
            >
              <ToolCallBlock
                :block="specimen.block"
              />
            </GallerySpecimen>
            <GallerySpecimen
              v-for="specimen in rowPictureLab"
              :key="specimen.variant"
              :variant="`row · ${specimen.variant}`"
              wide
            >
              <div v-for="size in rowPictureWidths" :key="size.name" :style="{ width: `${size.width}px`, maxWidth: '100%' }">
                <FunctionalBlock :expandable="true" :pictures="specimen.pictures" :icon-key="rowPictureRolled ? 'rolled' : 'stack'">
                  <template #icon>
                    <Layers :size="ICON_PX.in28" />
                  </template>
                  <span class="min-w-0 truncate">Ran 79 commands</span>
                  <Tag v-if="specimen.tagged" class="shrink-0 tabular-nums">1m 31s</Tag>
                </FunctionalBlock>
              </div>
            </GallerySpecimen>
            <Button size="sm" variant="ghost" @click="rowPictureRolled = !rowPictureRolled">Roll the Faces</Button>
            <GallerySpecimen variant="shell · twelve screenshots · narrow" wide>
              <div class="w-[420px]">
                <ToolCallBlock :block="twelveScreenshotsTool" />
              </div>
            </GallerySpecimen>
            <GallerySpecimen variant="pictures of a transcript · the viewer steps through them with the arrow keys" wide>
              <div class="h-[30rem]">
                <AgentMessageList
                  class="h-full"
                  conversation-id="gallery-viewer-transcript"
                  :blocks="viewerTranscript"
                  :pending-steers="[]"
                  :queue="[]"
                  phase="idle"
                  :bottom-offset="0"
                  :persisted-scroll-state="undefined"
                  read-only
                />
              </div>
            </GallerySpecimen>
            <GallerySpecimen variant="a run, folded, with twelve screenshots" wide>
              <div class="h-[11rem]">
                <AgentMessageList
                  class="h-full"
                  conversation-id="gallery-tool-media-run"
                  :blocks="screenshotRun"
                  :pending-steers="[]"
                  :queue="[]"
                  phase="idle"
                  :bottom-offset="0"
                  :persisted-scroll-state="undefined"
                  read-only
                />
              </div>
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="ActivitySlot"
        note="Requesting while the provider is asked, from the send of a message, its delivery included, and a Resume or Continue alike, so a slow model reads as the provider’s wait. The word and its clock stay through the server’s confirmation of the message and the agent’s own retries after a failed attempt: they are the same wait, and a row that changed its word with each attempt would flicker. Elapsed time shows after one second. What ends the wait, a step, the answer, a failure or a Stop, takes the row’s place and rolls over it, as Continue and Resume roll the Stopped row or the error record into it: the Turn specimens show each."
      >
        <div class="specimen-stack">
          <GallerySpecimen
            v-for="kind in activityKinds"
            :key="kind"
            :variant="kind"
            wide
          >
            <div class="gallery-frame gallery-activity-frame bg-surface">
              <ActivitySlot :kind="kind" :since="activitySince" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Calls Being Written"
        note="A call the model is still writing shows as its tool’s row, shimmering, after the transcript’s last block, from the moment its description is written, which every tool’s input lists first. Until then the transcript goes on showing what it showed, Requesting or the step before, so no placeholder title flashes. The call’s own block takes its place once the call is whole. A vendor that sends its calls only whole has nothing to show meanwhile either: the finished text is followed by Requesting, counting from the text’s end, so a running turn never looks finished."
      >
        <div class="specimen-stack">
          <GallerySpecimen
            v-for="specimen in writingSpecimens"
            :key="specimen.variant"
            :variant="specimen.variant"
            wide
          >
            <div class="gallery-frame h-[13rem] bg-surface">
              <AgentMessageList
                class="h-full"
                :conversation-id="`gallery-writing-${specimen.variant}`"
                :blocks="writingBlocks"
                :pending-calls="specimen.calls"
                :pending-steers="[]"
                :queue="[]"
                phase="running"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'lengths'">
      <GalleryUserMessageLengths :files="sessionFiles" :cwd="workspace.root" />
    </template>

    <template v-if="view === 'changes'">
      <GallerySection
        title="Work Groups"
        note="Consecutive steps show as one row. Only a step that has ended joins its run: a call once it returns, one whose command goes on in the background included, and thinking once it is written. What still runs, thinking being written, a call being written or waiting for its result, is a row of its own under the run, in the tail row's place: Requesting rolls into thinking, a call being written from its description, the call. When it ends it joins the run, and the next step, or Requesting while the turn waits for the model, takes the row. Only that row shimmers; the run is done. A run of one step reads and opens as that step; more read what they did, each kind of call once, in the order the run first made it, such as Ran 2 commands, checked 1 command, never a count of steps, and open to every step. A run says its total time in that sentence, running or ended, Ran 3 commands for 1m 32s: its steps' times added up, a call until it returned and a thinking until the next block, never the model's wait between steps; a run under a second names none. A step that joins never changes whether the run is open: one that joins an open run slides in as a row joining the transcript does, and a step the reader opened while it ran is still open as the run it starts. Thinking without text is covered by the step after it, and a run of thinking alone shows once a step that says something joins it. A command's notice and an agent's receipt between the steps are steps of the run too: the replay's background build reports its end and a child sends an update between two calls, and the run stays one row; the agent's text, the user's messages and steers, and the receipts of the user's decisions end a run. A run without a call reads what its notices and receipts told, such as 2 commands finished, heard from 1 agent. A failure shows on its own call inside. Opened, the notice and the receipt are rows of their own among the calls: the notice's title brings the build's call into view and marks the call's own row, or the run's row while it is folded, which stays folded; and the agent's name opens the Agents window at the call that sent the update. A command that has run more than 3 seconds wears the time on its own row, 1m32s, counting up, as in the run whose newest command runs, where End Command ends it and the tag goes, and Run Again starts it anew (Next Step walks a run from one step to its answer). A run whose command the agent left running wears that command's time after what it did, until the command ends. Folded, the pictures the calls and notices returned stand in the run's own line, in order, as a call's do, and the files the steps changed under it, each once; open, each call shows its own."
      >
        <div class="mb-3 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" @click="workFlow.play('work')">Replay</Button>
          <Button variant="ghost" size="sm" @click="workFlow.settleWork()">Show Ended</Button>
        </div>
        <GalleryReceipts :agents="workFlow.state.subagents" @abort="workFlow.abortSubagents" @abort-agent="workFlow.abortSubagent">
        <div class="gallery-frame h-[26rem] bg-surface">
          <SessionSurface ref="workSurface">
            <div class="flex h-full min-h-0 flex-col">
              <AgentMessageList
                ref="workList"
                class="min-h-0 flex-1"
                :conversation-id="workFlow.state.id"
                :blocks="workFlow.state.blocks"
                :pending-steers="[]"
                :queue="[]"
                :phase="workFlow.state.phase"
                :load="workFlow.state.load"
                :pending-action="workFlow.state.pendingAction"
                :pending-submissions="workFlow.pendingSubmissions.value"
                :removing="workFlow.removing.value"
                :pending-calls="workFlow.state.pendingCalls"
                :waiting-since="workFlow.state.waitingSince"
                :bottom-offset="workSurface?.dockHeight ?? 0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
            <template #dock>
              <SessionDock
                :show-scroll-to-bottom="!!workList && !workList.isAtBottom"
                @scroll-to-bottom="workList?.scrollToBottom()"
              />
            </template>
          </SessionSurface>
        </div>
        </GalleryReceipts>
        <GallerySpecimen variant="a run while its newest command runs" wide>
          <div class="gallery-frame h-[16rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-work-running"
              :blocks="runningRun.blocks"
              :pending-steers="[]"
              :queue="[]"
              phase="running"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
          <Button size="sm" variant="ghost" class="mt-1" @click="toggleRunningRun">
            {{ runningRun.command.phase === 'running' ? 'End Command' : 'Run Again' }}
          </Button>
        </GallerySpecimen>
        <GallerySpecimen :variant="`a run, step by step · ${STEPPED_RUN_STAGES[steppedStage]}`" wide>
          <div class="gallery-frame h-[16rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-work-stepped"
              :blocks="steppedBlocks"
              :pending-steers="[]"
              :queue="[]"
              :phase="steppedLast ? 'idle' : 'running'"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
          <Button size="sm" variant="ghost" class="mt-1" @click="nextStep">
            {{ steppedLast ? 'Start Over' : 'Next Step' }}
          </Button>
        </GallerySpecimen>
        <GallerySpecimen variant="an ended run whose command still runs" wide>
          <div class="gallery-frame h-[16rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-work-left-running"
              :blocks="leftRunning.blocks"
              :pending-steers="[]"
              :queue="[]"
              phase="idle"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
          <Button size="sm" variant="ghost" class="mt-1" @click="toggleLeftRunning">
            {{ leftRunning.command.phase === 'running' ? 'End Command' : 'Run Again' }}
          </Button>
        </GallerySpecimen>
        <GallerySpecimen variant="runs that check a command, or run one and check" wide>
          <div class="gallery-frame h-[34rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-work-looks"
              :blocks="lookBlocks"
              :pending-steers="[]"
              :queue="[]"
              phase="idle"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="a run that viewed images, then a report with media" wide>
          <div class="gallery-frame h-[30rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-work-media"
              :blocks="viewedBlocks"
              :pending-steers="[]"
              :queue="[]"
              phase="idle"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="A Heavy Turn"
        note="One refactor turn as the transcript shows it: a dozen shell calls in a row, most touching one or two files, one sweeping forty, the last still running."
      >
        <div class="gallery-frame h-[44rem] bg-surface">
          <SessionSurface ref="changesSurface">
            <div class="flex h-full min-h-0 flex-col">
              <AgentMessageList
                ref="changesList"
                class="min-h-0 flex-1"
                :conversation-id="changesFlow.state.id"
                :blocks="changesFlow.state.blocks"
                :pending-steers="[]"
                :queue="[]"
                :phase="changesFlow.state.phase"
                :load="changesFlow.state.load"
                :pending-action="changesFlow.state.pendingAction"
                :bottom-offset="changesSurface?.dockHeight ?? 0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
            <template #dock>
              <SessionDock
                :show-scroll-to-bottom="!!changesList && !changesList.isAtBottom"
                @scroll-to-bottom="changesList?.scrollToBottom()"
              />
            </template>
          </SessionSurface>
        </div>
      </GallerySection>
      <GallerySection
        title="Request’s Changes"
        note="A request runs from the user’s message to their next one, through command reports, receipts and steers. The pills under each call name its files, and a work group’s pills each file once; a pill opens the Change view below on its file at that call’s first edit, and a pill whose edit it already shows leaves it as it is; only the panel’s own control closes it. A subagent’s changes show only in its own transcript. With the changes plugin off, the pills are no controls. A conversation outside a project shows no pills; its Change view still lists the files."
      >
        <GallerySpecimen variant="two files, login.ts edited three times across a command report" wide>
          <div class="gallery-frame h-[30rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-request-sign-in"
              :blocks="signInBlocks"
              :pending-steers="[]"
              :queue="[]"
              phase="idle"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="still working · the same request before its reply, with no button yet" wide>
          <div class="gallery-frame h-[30rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-request-running"
              :blocks="signInRunningBlocks"
              :pending-steers="[]"
              :queue="[]"
              phase="running"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="contents not kept · the logo has none, the readme’s first edit none" wide>
          <div class="gallery-frame h-[22rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-request-uncopied"
              :blocks="uncopiedBlocks"
              :pending-steers="[]"
              :queue="[]"
              phase="idle"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="moved and removed in one run · the page written in /tmp and moved in; a script left in /tmp shows nowhere, and the scratch notes only under the calls that wrote and removed them" wide>
          <div class="gallery-frame h-[26rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-request-moved"
              :blocks="movedBlocks"
              :pending-steers="[]"
              :queue="[]"
              phase="idle"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="removed and renamed across two runs · the first run keeps the notes it made, the second strikes them and the old guide and shows the setup page renamed; the request lists the old guide and the new name, not the notes" wide>
          <div class="gallery-frame h-[24rem] bg-surface">
            <AgentMessageList
              class="h-full"
              conversation-id="gallery-request-cleanup"
              :blocks="cleanupBlocks"
              :pending-steers="[]"
              :queue="[]"
              phase="idle"
              :bottom-offset="0"
              :persisted-scroll-state="undefined"
              read-only
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="a subagent’s own request, beside its parent’s, which changed nothing" wide>
          <div class="grid grid-cols-2 gap-3">
            <div class="gallery-frame h-[22rem] bg-surface">
              <AgentMessageList
                class="h-full"
                conversation-id="gallery-request-parent"
                :blocks="parentBlocks"
                :pending-steers="[]"
                :queue="[]"
                phase="idle"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
            <div class="gallery-frame h-[22rem] bg-surface">
              <AgentMessageList
                class="h-full"
                conversation-id="gallery-request-subagent"
                :node="SUBAGENT"
                :blocks="childBlocks"
                :pending-steers="[]"
                :queue="[]"
                phase="idle"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="the changes plugin off · the line and the pills are no controls" wide>
          <GalleryEditSelection>
            <div class="gallery-frame h-[30rem] bg-surface">
              <AgentMessageList
                class="h-full"
                conversation-id="gallery-request-off"
                :blocks="signInOffBlocks"
                :pending-steers="[]"
                :queue="[]"
                phase="idle"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
          </GalleryEditSelection>
        </GallerySpecimen>
        <GallerySpecimen variant="a conversation outside a project · no pills" wide>
          <GalleryFilePills :shown="false">
            <div class="gallery-frame h-[30rem] bg-surface">
              <AgentMessageList
                class="h-full"
                conversation-id="gallery-request-outside"
                :blocks="signInOutsideBlocks"
                :pending-steers="[]"
                :queue="[]"
                phase="idle"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
          </GalleryFilePills>
        </GallerySpecimen>
      </GallerySection>
      <!-- The panel the pills above open; a click on what it already shows leaves it as it is, as in the product. -->
      <div v-if="editWork.open.value" class="h-[480px] overflow-hidden rounded-lg border border-line">
        <GalleryWorkPanel :work="editWork" @close="editWork.open.value = false" />
      </div>
      <div v-else class="flex items-center justify-between gap-3 rounded-lg border border-line py-2 pl-3 pr-2 text-sm text-fg-muted">
        <span>The work panel is closed. A pill above opens it again.</span>
        <Button size="sm" @click="editWork.open.value = true">Open Panel</Button>
      </div>
      <GallerySection
        title="Changed Files"
        note="A shell call lists the files it touched under its row: icon, name and line counts as pills that wrap. A new file carries a green plus, a removed one is struck through. Pick a file to show it in its request’s changes at that call’s first edit. The pills take at most two lines; the files that do not fit fold into a last pill, +N Files, which opens the request’s changes, where every file is listed. Resize the page to see it fold again. The row folds the command and output on its own; the pills do not move."
      >
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack [--agent-pad-x:0px]">
            <GallerySpecimen
              v-for="item in fileChangeCases"
              :key="item.block.id"
              :variant="item.variant"
              wide
            >
              <div :style="{ width: item.width ? `${item.width}px` : undefined, maxWidth: '100%' }">
                <GalleryTranscript :blocks="standaloneRequest(item.block)">
                  <ToolShellBlock
                    v-model:open="changeCaseOpen[item.block.id]"
                    :block="item.block"
                    :input="parseToolInput(item.block.input)"
                  />
                </GalleryTranscript>
              </div>
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'turns'">
      <GalleryStreamingText />
      <GalleryBusyRequest />
      <GalleryCommandWakeup />
      <GallerySection
        title="Turn"
        note="Requesting from the send, then each block rolls into the tail row. The message shows at once as it will stay, and its delivery is part of the same wait: when the server confirms it, the row keeps its word and its clock. Not Delivered fails the delivery instead: the failure under the message says why, with no button; the message’s hover actions offer Edit and Delete, a message sent next comes below it with its turn, and the dock’s Retry sends it again with the same ID, as a new message at the end, and shows Requesting from then. Offline sends while the socket is lost: the message shows as it will stay, the tail row says Connecting, never that it failed, and the message goes with its ID once the socket is back. Steer and Queue send a message while the command runs: it shows as the steer or the queued message it will be from the send, its line and Send Now and Recall in view, in the row and at the height it keeps once the server holds it, below the turn with nothing under it. Recall takes it back at once: into the composer when that is empty, the caret at its end; when the composer holds a draft, the draft stays and a Message Recalled toast offers Undo, which sends it again the same way. Send Now or Recall pressed before the server holds it is carried out once it does. Resume, Retry and Connect wait for the server first."
      >
        <div class="mb-3 flex flex-wrap gap-2">
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('turn')"
          >Replay</Button>
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('undelivered')"
          >Not Delivered</Button>
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('offline')"
          >Offline</Button>
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('resume')"
          >Resume</Button>
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('retry')"
          >Retry</Button>
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('connect')"
          >Connect</Button>
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('steer')"
          >Steer</Button>
          <Button
            variant="ghost"
            size="sm"
            @click="playTurn('queue')"
          >Queue</Button>
        </div>
        <div class="gallery-frame h-[28rem] bg-surface">
          <SessionSurface ref="turnSurface">
            <div class="flex h-full min-h-0 flex-col">
              <AgentMessageList
                ref="turnList"
                class="min-h-0 flex-1"
                :conversation-id="turnFlow.state.id"
                :blocks="turnFlow.state.blocks"
                :pending-steers="turnFlow.state.pendingSteers"
                :queue="turnFlow.state.queue"
                :phase="turnFlow.state.phase"
                :load="turnFlow.state.load"
                :pending-action="turnFlow.state.pendingAction"
                :pending-submissions="turnFlow.pendingSubmissions.value"
                :removing="turnFlow.removing.value"
                :bottom-offset="turnSurface?.dockHeight ?? 0"
                :persisted-scroll-state="undefined"
                read-only
                @edit-submission="turnFlow.editSubmission"
                @delete-submission="turnFlow.deleteSubmission"
                @recall-steer="turnFlow.recallSteer"
                @interrupt-pending-steer="turnFlow.interruptPendingSteer"
                @recall-queued="turnFlow.recallQueued"
                @send-queued="turnFlow.sendQueued"
              />
            </div>
            <template #dock>
              <SessionDock
                :show-scroll-to-bottom="!!turnList && !turnList.isAtBottom"
                @scroll-to-bottom="turnList?.scrollToBottom()"
              >
                <template #chips>
                  <UndeliveredRetryChip :pending-submissions="turnFlow.pendingSubmissions.value" @retry="turnFlow.retrySubmissions()" />
                </template>
                <GalleryComposer
                  :running="turnFlow.state.phase === 'running' || turnFlow.startingTurn.value"
                  :compacting="turnFlow.state.phase === 'compacting'"
                  :flow="turnFlow"
                  @send="turnFlow.turn"
                  @steer="turnFlow.steer"
                  @queue="turnFlow.queue"
                  @stop="turnFlow.stop"
                  @compact="turnFlow.compact"
                />
              </SessionDock>
            </template>
          </SessionSurface>
        </div>
      </GallerySection>

      <GallerySection
        title="Stream"
        note="Thinking then reply with nothing waited for: the reveal alone."
      >
        <div class="mb-3">
          <Button
            variant="ghost"
            size="sm"
            @click="playStream"
          >Replay</Button>
        </div>
        <div class="gallery-frame h-[20rem] bg-surface">
          <SessionSurface ref="streamSurface">
            <div class="flex h-full min-h-0 flex-col">
              <AgentMessageList
                ref="streamList"
                class="min-h-0 flex-1"
                :conversation-id="streamFlow.state.id"
                :blocks="streamFlow.state.blocks"
                :pending-steers="[]"
                :queue="[]"
                :phase="streamFlow.state.phase"
                :load="streamFlow.state.load"
                :pending-action="streamFlow.state.pendingAction"
                :bottom-offset="streamSurface?.dockHeight ?? 0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
            <template #dock>
              <SessionDock
                :show-scroll-to-bottom="!!streamList && !streamList.isAtBottom"
                @scroll-to-bottom="streamList?.scrollToBottom()"
              />
            </template>
          </SessionSurface>
        </div>
      </GallerySection>
      <GallerySection
        title="Compaction"
        note="The user compacted after the second answer. The divider shows there, where the compaction was triggered, though the summary goes in before that answer. While the pass runs the divider says so, its words shimmering as a running row’s do, before a steer that arrived meanwhile, and the finished divider rolls in over it; a failed pass leaves its error record there. A Compact the user asked for ended no turn, so the dock offers no Resume for it (Session failures shows both cases)."
      >
        <div class="specimen-stack">
          <GallerySpecimen
            v-for="item in compactionCases"
            :key="item.state"
            :variant="item.variant"
            wide
          >
            <div class="gallery-frame h-[16rem] bg-surface">
              <AgentMessageList
                class="h-full"
                :conversation-id="`compaction-${item.state}`"
                :blocks="compactionTranscript(item.state)"
                :pending-steers="item.state === 'running' ? compactionSteers : []"
                :queue="[]"
                :phase="item.state === 'running' ? 'compacting' : 'idle'"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
                @recall-steer="productWould('Recall the Steer')"
                @interrupt-pending-steer="productWould('Send the Steer Now')"
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'shared'">
      <GallerySharedPage />
    </template>
    <template v-if="view === 'windows'">
      <GallerySection
        title="Agents"
        note="Half-session inspect. Tabs are running children; a click only selects one, and nothing on a tab stops it. A right-click on a tab, or the Menu key or Shift+F10 on a focused one, opens its menu: Stop Agent for a running child, Copy Agent ID for any (it copies the number the `demi agent` commands take, and a toast says Copied), Close Tab for a finished one, the other of the two disabled. A child's last row reads its own phase: Find the cookie assertion ended its turn while its search runs on, so it shows no Requesting row, while Check the logout path waits for its model. Stop Agent stops the child at once; a command it still runs stays in the Running window as Stopping…, its tab's mark a spinner, until its runner says that it ended, here 3 seconds later: stop Find the cookie assertion and open the Terminals window below. The More button at the strip’s end, and a right-click on the strip’s empty area, open the strip’s menu: Stop All, disabled once nothing runs. Along the window's bottom edge a thin bar, in the secondary text color under its own divider, says what the shown tab's child is doing and runs on: its state and how long it has run, counting while it runs, Forked from agent N for a fork, and on the right its profile, model, effort and Fast (What an Agent Runs On), then the composer's context indicator for the child's own context, its segments running close to the window's ends, the first and last text as far from the window's side as its corner is round. Its card is the composer's, filled from the child, with the usage limits of the account the child runs on; its Compact compacts that child, here a toast, and waits while the child's turn runs. Completed opens the searchable finished list, each child's profile a tag after its description, none for no profile, then how long it ran; the child picked there shows as a tab after the running ones, and picking another replaces it in place, without motion. With no child running the window opens empty, never on a child the user did not pick; closing the last finished tab leaves it empty. It stands with the media viewer a screenshot in a child’s transcript opens in the order they opened, the newest in front: a press outside every window, here anywhere on the page but a window, closes the newest alone, as Escape does, and a second press the next. Close (×), or Escape with the focus in it, closes it too, and Run Them Again opens it again. Its top edge resizes it: drag it, or focus it and press the arrow keys (Shift for bigger steps, Home and End for the smallest and the largest), and a double-click returns to the usual height, half the area. The height is kept as a share of the area, one for every window here, as the product keeps one for its Agents and Running windows; it never goes under the strip, the bar and three lines of the transcript, nor over the room above the dock. On a phone the tabs of this window and of Running narrow to their icons and status dots, the selected one adding its name (Phone page)."
      >
        <div class="mb-3 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" @click="resetWindows">Run Them Again</Button>
        </div>
        <DockWindowArea v-model:share="dockWindowShare" class="h-[24rem]">
          <SubagentPanel
            v-model:open="exhibitOpen"
            v-model:active-id="exhibitAgentId"
            :agents="agents"
            @abort="sessionFlow.abortSubagents"
            @abort-agent="sessionFlow.abortSubagent"
            v-bind="agentContext.panel"
          />
        </DockWindowArea>
      </GallerySection>
      <GallerySection
        title="Working Directory"
        note="A child works in a directory of its own (What an Agent Runs On): here it was spawned into another checkout of the project, demi-slots/2, and moved into src/auth. The page reads every relative path against the directory the child wrote it in, so its first link, src/auth/cookie.ts, opens the file of its checkout and not the conversation's file of the same name, and its second, cookie.ts, written after the move, opens the same file by its short name. The File view opens for that directory: its path bar shows the whole path and its tree is the checkout's. The file pill under the child's edit opens the Change view for the same directory, and the folder in the bar's left group opens the File view on the directory's tree. The panel's own tabs, here File, stay on the conversation's directory until a link of a child opens them for its own."
      >
        <GalleryAgentDirectory />
      </GallerySection>
      <GallerySection
        title="Loading an Agent"
        note="A conversation opens without its agents' transcripts, so the Agents window may show a child the page holds none of: a live child's transcript comes from its session once the window shows it, and a finished child's latest page is read. Until it is there the window says Loading agent… in place of the transcript, never an empty one; Finish Loading here brings the live child's transcript. A read that fails says the agent couldn’t load, with the reason under it and Retry, which loads again: here Retry's read brings the transcript a moment later, and Fail Again shows the failure anew. The status bar stays under either, since it reads the child's record, not its transcript."
      >
        <GalleryAgentLoading />
      </GallerySection>
      <GallerySection
        title="What an Agent Runs On"
        note="The Agents window's bar says what a child is doing and what it runs on in segments, as an editor's status bar does: each item stands in its own segment, told apart by spacing, never by dividers. A segment that acts when pressed, the folder, Forked from agent N and the context indicator, shows a hover background over its whole extent, reaching to within the bar's small inset of the window's edge, the same on every side it touches, and Tab reaches it with a focus ring (one specimen pins that background); a segment that only explains, the state, the profile and the model, shows its tip and no background. Two groups say what a child is doing and what it runs on. On the left: the child's state dot, the one its tab wears, the state, Running, Finished, Stopped or Failed, how long the round has run, in the duration words of the reset times (42s, 16m 48s, 1h 5m) and counting with the page's clock while it runs, the folder it works in, its own name with the whole path as its tip, which opens the File view on that directory's tree (a toast here, naming the directory; the next section opens a real panel), and Forked from agent N for a fork, a link to that agent: its mark and the click of a message's link, which opens the Agents window on its tab here, shown in its own segment. The round is not shown. On the right: the profile as a tag, none for the inherit profile; the model's name as the model menu shows it, pointed at naming the provider and the full name, and for the inherit profile Inherited from this conversation; the effort, or Thinking off; Fast when it is on. Where the room runs out, Forked from gives way first, then the model's name and the folder's name, which leaves the folder's icon and its tip; the state and the time always stay (the last three specimens, by width). The Agents window shows it as a bar along its bottom edge (an ended child's bar has no context indicator: finished, stopped and failed show none)."
      >
        <GalleryReceipts :agents="agents" @abort="sessionFlow.abortSubagents" @abort-agent="sessionFlow.abortSubagent">
          <GallerySpecimen v-for="run in runLines" :key="run.id" :variant="runLineVariants[run.id] ?? run.name" wide>
            <AgentStatusBar
              :run="run"
              :usage="run.contextUsage"
              v-bind="agentCompaction(run)"
              :instructions="[]"
              :limits="agentContext.limits.value"
              @compact="agentContext.compact"
              @open-instruction="agentContext.openInstruction"
              @refresh-limits="agentContext.refresh"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="pressable segments pinned hovered · the folder, Forked from and the context indicator">
            <AgentStatusBar
              class="w-[46rem] [&_[data-acting]]:bg-hover"
              :run="runLines[2]!"
              :usage="runLines[2]!.contextUsage"
              v-bind="agentCompaction(runLines[2]!)"
              :instructions="[]"
              :limits="agentContext.limits.value"
              @compact="agentContext.compact"
              @open-instruction="agentContext.openInstruction"
              @refresh-limits="agentContext.refresh"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="room for everything · 46rem">
            <AgentStatusBar
              class="w-[46rem]"
              :run="runLines[2]!"
              :usage="runLines[2]!.contextUsage"
              v-bind="agentCompaction(runLines[2]!)"
              :instructions="[]"
              :limits="agentContext.limits.value"
              @compact="agentContext.compact"
              @open-instruction="agentContext.openInstruction"
              @refresh-limits="agentContext.refresh"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="narrower · Forked from gives way · 36rem">
            <AgentStatusBar
              class="w-[36rem]"
              :run="runLines[2]!"
              :usage="runLines[2]!.contextUsage"
              v-bind="agentCompaction(runLines[2]!)"
              :instructions="[]"
              :limits="agentContext.limits.value"
              @compact="agentContext.compact"
              @open-instruction="agentContext.openInstruction"
              @refresh-limits="agentContext.refresh"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="narrowest · the model's name gives way · 20rem">
            <AgentStatusBar
              class="w-[20rem]"
              :run="runLines[2]!"
              :usage="runLines[2]!.contextUsage"
              v-bind="agentCompaction(runLines[2]!)"
              :instructions="[]"
              :limits="agentContext.limits.value"
              @compact="agentContext.compact"
              @open-instruction="agentContext.openInstruction"
              @refresh-limits="agentContext.refresh"
            />
          </GallerySpecimen>
        </GalleryReceipts>
      </GallerySection>
      <GallerySection
        title="A Child's Brief"
        note="A child's task brief, the message its spawn wrote, is Markdown an agent wrote: it renders as an answer does, headings, lists and tables included, inside the bubble that marks it as the child's input, folded behind Show More when long. Copy puts its Markdown on the clipboard; it has no Edit. What the user types keeps the user dialect (UserBlock). Close (×) closes the window and Open Again opens it; Stop Agent and Stop All stop the child."
      >
        <div class="mb-3 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" :disabled="briefOpen" @click="briefOpen = true">Open Again</Button>
        </div>
        <DockWindowArea v-model:share="dockWindowShare" class="h-[32rem]">
          <SubagentPanel
            v-model:open="briefOpen"
            v-model:active-id="briefAgentId"
            :agents="briefAgents"
            @abort="stopBriefAgent"
            @abort-agent="stopBriefAgent"
            v-bind="agentContext.panel"
          />
        </DockWindowArea>
      </GallerySection>
      <GallerySection
        title="Agents Chip"
        note="The dock's Agents chip opens this window and a second click closes it. A press on the reply above closes it, as a press in the conversation or the sidebar outside every window does, the message field and the empty row beside the chip included; a press on the chip is its own control, while one on Resume, which opens no window, closes the window and resumes; typing with the caret in the message closes nothing. Open a screenshot in the first child's transcript: the viewer opens over the window, and the first press on the reply closes only the viewer, the next the window. While children run it counts them and the window opens on the oldest running one. Once none runs there is no chip: Stop All in the window ends them, and the window, still open, keeps the tab it showed; closing that tab leaves it saying no agent is running. A finished child opens from a link to it in the reply, on its tab, as Completed picks it. Run Them Again starts the children again."
      >
        <div class="mb-3 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" @click="resetWindows">Run Them Again</Button>
        </div>
        <div class="grid grid-cols-1 gap-3 xl:grid-cols-2">
          <GallerySpecimen variant="children running" wide>
            <GalleryAgentsChip
              :agents="agents"
              :terminals="terminals"
              @abort="sessionFlow.abortSubagents"
              @abort-agent="sessionFlow.abortSubagent"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="none running" wide>
            <GalleryAgentsChip
              :agents="finishedOnly"
              :terminals="terminals"
              @abort="sessionFlow.abortSubagents"
              @abort-agent="sessionFlow.abortSubagent"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Terminals"
        note="The same window. Tabs are running jobs, each titled by its call’s description, cut at the end and whole in its tooltip; the body is a read-only xterm with ANSI color from bun, rg and git. The terminal opens with the script as a terminal shows what was typed: a muted $ and the first line, a muted > before each further line (Show the auth test changes), long lines wrapped at the terminal’s width (Find where the old cookie name is still used), the script highlighted as Bash in the colors all code shows in (Count the old cookie name and list the changed files), then the output. The watch tab’s output comes live: the terminal adds only what is new and keeps its scrollback, and after a burst longer than a frame holds, it shows the frame’s tail anew. A click on a tab only selects it; its menu, from a right-click or the Menu key or Shift+F10, ends a running command (End Command) or closes an ended one’s tab (Close Tab). A command being stopped shows a spinner in its tab’s mark until its runner says that it ended, and End Command is off meanwhile. A press outside every window closes it while it is the newest; Close (×), or Escape with the focus on its strip, closes it too, while the terminal keeps the Escape typed into it. Its top edge resizes it, and shares its height with the Agents window above."
      >
        <div class="mb-3 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" @click="resetWindows">Run Them Again</Button>
        </div>
        <DockWindowArea v-model:share="dockWindowShare" class="h-[24rem]">
          <TerminalPanel
            v-model:active-id="exhibitTerminalId"
            :terminals="terminals"
            @abort="sessionFlow.abortTerminal"
          />
        </DockWindowArea>
      </GallerySection>
      <GallerySection
        title="Panel Under a Permission Card"
        note="A panel the dock opened keeps its size and place while a permission request waits: the card stands over the panel’s lower part, above it, with its own edge and shadow, and the panel scrolls and works above the card’s top. Nothing moves when the card comes or goes (Decide, Ask Again). The transcript behind keeps clear of the card when no panel is open (the Agents chip closes and opens the panel)."
      >
        <div class="mb-3 flex flex-wrap gap-2">
          <Button variant="ghost" size="sm" :disabled="coverRequests.length > 0" @click="coverRequests = [rootRequest()]">Ask Again</Button>
        </div>
        <div class="gallery-frame h-[40rem] bg-surface">
          <SessionSurface ref="coverSurface" v-model:dock-window-share="dockWindowShare">
            <div class="flex h-full min-h-0 flex-col">
              <AgentMessageList
                class="min-h-0 flex-1"
                conversation-id="gallery-cover"
                :blocks="coverBlocks"
                :pending-steers="[]"
                :queue="[]"
                phase="idle"
                :bottom-offset="coverSurface?.dockHeight ?? 0"
                :persisted-scroll-state="undefined"
                read-only
              />
            </div>
            <template #dock>
              <SessionDock>
                <template v-if="coverRequests.length" #above>
                  <RequestCard
                    :requests="coverRequests"
                    @decide="(id, decision) => (coverRequests = decidePermission(coverRequests, id, decision))"
                  />
                </template>
                <template #chips>
                  <AgentsChip
                    :agents="agents"
                    :open="coverOpen"
                    @open="coverOpen = !coverOpen; coverAgentId = firstRunningSubagentId(agents)"
                  />
                </template>
                <GalleryComposer />
              </SessionDock>
            </template>
            <template #overDock>
              <SubagentPanel
                v-model:open="coverOpen"
                v-model:active-id="coverAgentId"
                :agents="agents"
                :terminals="terminals"
                @abort="sessionFlow.abortSubagents"
                @abort-agent="sessionFlow.abortSubagent"
                v-bind="agentContext.panel"
              />
            </template>
          </SessionSurface>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'states'">
      <GalleryHistoryWindow />
      <GallerySection title="Transcript Entrance" note="Restored history appears without motion. Only blocks appended after restoration enter; switching or scrolling back to existing blocks does not replay their entrance.">
        <GalleryTranscriptEntrance />
      </GallerySection>
      <GallerySection title="Edit and Resend" note="Shared editor, durable confirmation and recovery states. Regenerate in the last answer’s footer sends the last message again unchanged, without opening the editor: its answer dims until the outcome, which the buttons choose as for a save; a lost confirmation opens the editor with Retry, a conflict opens it with the message. Up Arrow in the empty composer opens the editor on the last message. An edited message shows its versions under it, and a regenerated answer in its footer: the arrows show the version before and after, and wait while an edit is sent.">
        <GalleryMessageEditing />
      </GallerySection>
      <GallerySection title="Comments" note="Two comments waiting in the draft: each marks its text yellow, with a comment mark in the reply's margin, and the composer shows 2 Comments inside its box. Pointing at a marked text or its mark shows the comment with Delete and Edit; a click pins it. Select other words of the reply and choose Comment in the toolbar above them, write one and press Return to add it; while a reply is still being written, Comment says to wait. The pill lists the waiting comments, each with its quote, and × removes them all. Send sends them with the typed text, or alone.">
        <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
          <GalleryComments seed="waiting" />
        </div>
      </GallerySection>
      <GallerySection title="Sent Comments" note="A message sent with two comments: its bubble shows each quoted text as a quote, its comment under it, then the typed text, and the reply keeps their marks, dimmer; their card says Sent. A share's page shows the same bubble and no marks.">
        <div class="h-[40rem] overflow-hidden rounded-xl border border-line">
          <GalleryComments seed="sent" />
        </div>
      </GallerySection>
      <GallerySection title="Comment Box" note="The box a new comment is written in: Comment, the default, adds it to the waiting comments (Return); Send Now sends a message holding this comment alone at once (⌘Return, Control-Return elsewhere), leaving the other waiting comments and the typed draft as they are, and steering or waiting in the queue while the agent works. Shift-Return makes a new line, Escape cancels. A side question's box keeps its single send.">
        <GalleryCommentBox />
      </GallerySection>
      <GallerySection title="Comment on a Picture" note="A picture in the reply takes a comment from its menu (right-click, or a long press on a phone): Copy Image and Save Image, then Comment, Ask and Send to Conversation. The comment outlines the picture yellow and puts its mark in the margin at the picture's top; pointing at the picture or the mark shows the comment. It points at the picture by its block and its place among the block's pictures, and keeps the picture itself as its quote.">
        <div class="h-[40rem] overflow-hidden rounded-xl border border-line">
          <GalleryComments seed="picture-waiting" />
        </div>
      </GallerySection>
      <GallerySection title="Sent Comment on a Picture" note="A message sent with a comment on a picture: its bubble shows a small copy of the picture as the quote, then the comment; the model reads the picture as an image the message carries, then the comment. The picture's outline stays, dimmer.">
        <div class="h-[40rem] overflow-hidden rounded-xl border border-line">
          <GalleryComments seed="picture-sent" />
        </div>
      </GallerySection>
      <GallerySection title="Ask" note="A side question on a reply's words, live from scratch: select words of the reply and choose Ask in the toolbar, after Comment, write a question and press Return. The words turn purple with a purple underline, and a card opens under their block on the page's background, its border the line color: a purple question mark, the title in secondary text, Ask until the small model's title arrives, and two ghost icon buttons, ↗ Open Full Conversation and × Collapse; then the question in bold and one status line while the answer is made, the step it runs, and the short answer. The conversation's own agent never learns of it. Hovering the words deepens them. The answers are scripted.">
        <div class="h-[40rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="live" />
        </div>
      </GallerySection>
      <GallerySection title="Ask · Asking" note="Just asked: the card is titled Ask until the small model's title arrives, and its one status line says it thinks. Reply still asks a follow-up, × folds it and ↗ opens the side conversation.">
        <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="asking" />
        </div>
      </GallerySection>
      <GallerySection title="Ask · Working" note="Titled, while it works: the one status line names the step it runs, never the steps themselves; ↗ shows them all in the subagent window.">
        <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="working" />
        </div>
      </GallerySection>
      <GallerySection title="Ask · Answered" note="The question in bold and its short answer, with a bordered Reply below it.">
        <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="answered" />
        </div>
      </GallerySection>
      <GallerySection title="Ask · Follow-Up" note="Reply opened a field with a send button and a follow-up typed: Return sends it as the side conversation's next turn, Shift-Return makes a new line, Escape cancels; the card's other controls still do what they say, and an empty field closes when a click lands outside the card.">
        <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="follow-up" />
        </div>
      </GallerySection>
      <GallerySection title="Ask · Folded" note="× folded the card into a purple question mark in the reply's right margin at the asked line; a click on the mark or on the purple words opens the card again, scrolled into view.">
        <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="folded" />
        </div>
      </GallerySection>
      <GallerySection title="Ask · Failed" note="A failed answer shows the failure's words and its ID, as a turn's failure does, with Retry, which runs the turn again.">
        <div class="h-[36rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="failed" />
        </div>
      </GallerySection>
      <GallerySection title="Ask on a Picture" note="Ask from a picture's menu starts a side question whose first message carries the picture as its quote: the picture is outlined in purple and the card opens under it; ↗ shows the picture and the question as the side conversation's first message.">
        <div class="h-[40rem] overflow-hidden rounded-xl border border-line">
          <GalleryAsks seed="picture" />
        </div>
      </GallerySection>
      <GallerySection title="Ask on a Phone" note="On a phone the selection's toolbar holds Comment, Ask and Send to Conversation; the card spans the reply's width, and ↗ opens the side conversation full screen. Each phone is live.">
        <div class="flex flex-wrap gap-8">
          <GalleryPhoneFrame path="/asks-frame" scene="answered" label="An answered side question" active />
          <GalleryPhoneFrame path="/asks-frame" scene="folded" label="Folded into the margin" active />
        </div>
      </GallerySection>
      <GallerySection
        title="Product Session"
        note="The shared product page, with local fixture state and handlers. The header gives its title up to 260px and cuts it short last: a longer title is cut to that and the buttons keep their names, and as the frame narrows, the directory button and then the host button become their icons, their names in tooltips, and they name themselves again once the title fits whole. Resize the frame from its corner. The directory button lists the projects on the Host, since every directory a conversation runs in but the home is one; moving to another folder makes it a project there, and the home lists none. Each is read by its own name with where it is beside it, smaller and grey (the home written ~), the whole path in its tooltip; two directories of one name, demi in ~/Projects and in ~/Work, differ by it, and a long location gives up its left. The Rename button beside the title edits it in place, as a double-click on the title does: Enter or clicking away keeps the new title, Escape or an empty one keeps the old."
      >
        <div class="max-w-full resize-x overflow-hidden pb-3" style="min-width: 20rem">
          <GalleryConnectedSession />
        </div>
      </GallerySection>
      <GallerySection
        title="Scroll Control Without Task Chips"
        note="Scroll up, then return to the bottom. The left-aligned arrow fades inside a permanent control row. Transcript padding includes that row even when the arrow is hidden."
      >
        <GalleryConnectedSession :show-activity="false" />
      </GallerySection>
      <GallerySection
        title="Session Load"
        note="First opening uses the loading pane until history and agents arrive, then shows them before its session opens. History stays readable while models load or the connection opens. Switching back to a cached session is immediate and reuses its connection. A dropped connection in an open session uses the “Connecting” tail row. Enter while the models load says why on the send button, and the message goes once they have (Finish Loading). An unconfirmed send keeps its user message as it will stay, and the next message can be sent meanwhile, after it; Retry sends it again with the same ID and says Requesting from then, through the confirmation, until the answer begins. New conversation opens a local draft immediately."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen variant="unconfirmed send · retry the same message" wide>
            <div class="gallery-frame flex h-[16rem] flex-col overflow-hidden bg-surface">
              <AgentMessageList
                class="min-h-0 flex-1"
                :conversation-id="deliveryFlow.state.id"
                :blocks="deliveryFlow.state.blocks"
                :pending-steers="[]"
                :queue="[]"
                :phase="deliveryFlow.state.phase"
                :load="deliveryFlow.state.load"
                :pending-submissions="deliveryFlow.pendingSubmissions.value"
                :removing="deliveryFlow.removing.value"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
                @edit-submission="deliveryFlow.editSubmission"
                @delete-submission="deliveryFlow.deleteSubmission"
              />
            </div>
            <div class="mt-2 flex items-center gap-2">
              <UndeliveredRetryChip :pending-submissions="deliveryFlow.pendingSubmissions.value" @retry="deliveryFlow.retrySubmissions()" />
              <Button size="sm" @click="failDelivery">Fail Again</Button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="loading"
            wide
          >
            <div class="gallery-frame h-[16rem] bg-surface">
              <SessionStatus kind="loading" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="history ready · models loading" wide>
            <div class="gallery-frame flex h-[24rem] flex-col overflow-hidden bg-surface">
              <div class="min-h-0 flex-1">
                <AgentMessageList
                  conversation-id="history-without-models"
                  :blocks="historyModelBlocks"
                  :pending-steers="[]"
                  :queue="[]"
                  phase="idle"
                  load="ready"
                  :bottom-offset="0"
                  :persisted-scroll-state="undefined"
                  read-only
                />
              </div>
              <!-- The composer stands in the dock's 12px, as the session's does. -->
              <div class="px-3 pb-3">
                <GalleryComposer
                  :providers="historyModelsLoaded ? undefined : []"
                  :models="historyModelsLoaded ? undefined : {}"
                  :model-load="historyModelsLoaded ? 'ready' : 'loading'"
                  @send="productWould('Send the Message')"
                />
              </div>
            </div>
            <Button size="sm" class="mt-2" @click="historyModelsLoaded = !historyModelsLoaded">
              {{ historyModelsLoaded ? 'Load Models Again' : 'Finish Loading' }}
            </Button>
          </GallerySpecimen>
          <GallerySpecimen variant="opening · transcript, then the session; the agents when listed" wide>
            <p class="pb-2 text-[12px] leading-4 text-fg-muted">
              A conversation the page opens shows in two steps. Read History: the transcript appears with the composer as soon as it and the draft are read, waiting for nothing else; the turn is not known yet, so no tail row, Running or Resume shows, and the sidebar keeps the summary’s status. Open Session: the session’s turn and its running command join in one change. The agents join whenever their list arrives, before the session or after it: List the Agents brings everything that names them, the Agents chip among them. Lose the Socket shows a first opening that cannot connect: it says it connects, never a session it never had.
            </p>
            <GalleryOpeningSteps />
          </GallerySpecimen>
          <GallerySpecimen variant="switching · cached sessions" wide>
            <GalleryCachedSessions />
          </GallerySpecimen>
          <GallerySpecimen
            variant="reconnecting"
            wide
          >
            <div class="gallery-frame gallery-activity-frame bg-surface">
              <ActivitySlot kind="connecting" :since="activitySince" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="reconnecting · cached"
            wide
          >
            <div
              class="gallery-frame flex h-[16rem] flex-col overflow-hidden bg-surface"
            >
              <ScrollArea class="flex-1" viewport-class="pt-2">
                <UserBlock :content="userBubble" />
                <ActivitySlot kind="connecting" :since="activitySince" />
              </ScrollArea>
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="failed · live"
            wide
          >
            <div class="mb-2">
              <Button
                variant="ghost"
                size="sm"
                :disabled="liveLoad === 'failed'"
                @click="breakSession"
                >Break</Button
              >
            </div>
            <div
              class="gallery-frame flex h-[16rem] flex-col overflow-hidden bg-surface"
            >
              <SessionStatus
                v-if="livePane"
                :kind="livePane"
                :detail="
                  livePane === 'failed' ? requestFailure('The connection was reset.') : undefined
                "
                @retry="retrySession"
              />
              <ScrollArea
                v-else
                class="flex-1"
                viewport-class="pt-2"
              >
                <UserBlock :content="userBubble" />
              </ScrollArea>
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="empty"
            wide
          >
            <div class="gallery-frame h-[16rem] bg-surface">
              <SessionStatus kind="empty" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="none · no conversation open"
            wide
          >
            <div class="gallery-frame h-[16rem] bg-surface">
              <SessionStatus
                kind="none"
                @create="showToast({ title: 'Start a Conversation', message: 'The product opens a new conversation here.', tone: 'neutral' })"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="offline · not kept for opening offline"
            wide
          >
            <div class="gallery-frame h-[16rem] bg-surface">
              <SessionStatus kind="offline" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="missing · live"
            wide
          >
            <div class="mb-2">
              <Button
                variant="ghost"
                size="sm"
                :disabled="missingKind === 'missing'"
                @click="missingKind = 'missing'"
                >Break</Button
              >
            </div>
            <div class="gallery-frame h-[16rem] bg-surface">
              <SessionStatus
                :kind="missingKind"
                @create="missingKind = 'empty'"
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'panel'">
      <GallerySection
        title="The Frame"
        note="The app frame with the work panel open on the right: the sidebar, the session, and the panel are siblings, each side pane behind its own divider. The header’s panel control opens it and the panel’s fold control closes it, using the same 28px button, 14px icon, 12px right inset, tooltip and hover treatment as the conversation’s Open panel control; drag or double-click the divider on its left. A press in the panel or on its divider leaves an open Agents or Running window open, so a child’s changes are read beside it; only a press in the sidebar or the session closes it. The panel keeps a share of the width it splits with the session, so resizing the frame scales both while the sidebar keeps its px width; Reset Panel restores the initial selections."
      >
        <GallerySpecimen variant="frame · live" wide>
          <!-- The well holds what the frame fixes to the window on a phone, the work panel's
               sheet and the sidebar's drawer, so they cover this frame, not the specimens below. -->
          <GalleryOverlayWell size="tall" class="w-full">
          <div class="gallery-frame flex h-[44rem] w-full overflow-hidden">
            <SidebarLayout
              v-model:width="panelSidebarWidth"
              v-model:aside-open="panelAsideOpen"
              v-model:aside-share="panelAsideShare"
              class="w-full"
            >
              <template #sidebar>
                <AppSidebar
                  :new-shortcut="APP_SHORTCUTS.find((shortcut) => shortcut.id === 'new')?.keys"
                  :search-shortcut="APP_SHORTCUTS.find((shortcut) => shortcut.id === 'search')?.keys"
                  @search="frameSearchOpen = true"
                  :account="demoAccount"
                  :projects="panelProjects"
                  :conversations="panelConversations"
                  :active-id="panelActiveConversationId"
                  :section-entries="sidebarEntries(PLUGIN_PAGES, () => true)"
                  @select="(id) => (panelActiveConversationId = id)"
                  @archive="(ids) => (panelConversations = panelConversations.filter((conversation) => !ids.includes(conversation.id)))"
                  @delete="(ids) => (panelConversations = panelConversations.filter((conversation) => !ids.includes(conversation.id)))"
                  @open-settings="(section) => productWould(section ? `Open ${section} Settings` : 'Open Settings')"
                  @hand-off-to-phone="productWould('Open Hand Off to Your Phone')"
                  @sign-out="productWould('Sign Out')"
                />
              </template>
              <ChatSession @show-subagents="() => {}"
                :conversation="session"
                has-provider
                :reveal="frameReveal"
                @revealed="frameReveal = null"
                @reveal="(at) => (frameReveal = at)"
                :aside-open="panelAsideOpen"
                :panel-tabs="panelWork.panel.value.tabs.length"
                :share="shareService"
                :shares="sessionShares"
                :select-edit="panelWork.selectEdit"
                :files="sessionFiles"
                @open-aside="panelAsideOpen = true"
                :pending-submissions="sessionFlow.pendingSubmissions.value"
                :removing="sessionFlow.removing.value"
                @retry-undelivered="sessionFlow.retrySubmissions()"
                @edit-submission="sessionFlow.editSubmission"
                @delete-submission="sessionFlow.deleteSubmission"
                @retry="sessionFlow.resume()"
                @rename="session.title = $event"
                @abort-subagents="sessionFlow.abortSubagents"
                @abort-subagent="sessionFlow.abortSubagent"
                @abort-terminal="sessionFlow.abortTerminal"
                @recall-queued="sessionFlow.recallQueued"
                @send-queued="sessionFlow.sendQueued"
                @recall-steer="sessionFlow.recallSteer"
                @interrupt-pending-steer="sessionFlow.interruptPendingSteer"
                v-bind="agentContext.session"
              >
                <template #composer>
                  <GalleryComposer
                    placeholder="Ask Demi about the failing login test…"
                    :running="session.phase === 'running' || sessionFlow.startingTurn.value"
                    :compacting="session.phase === 'compacting'"
                    :usage="session.contextUsage ?? undefined"
                    :flow="sessionFlow"
                    @send="sessionFlow.turn"
                    @steer="sessionFlow.steer"
                    @queue="sessionFlow.queue"
                    @stop="sessionFlow.stop"
                    @compact="sessionFlow.compact"
                  />
                </template>
              </ChatSession>
              <template #aside>
                <GalleryWorkPanel :work="panelWork" @close="panelAsideOpen = false" />
              </template>
              <template #dialogs>
                <SearchDialog
                  :is-open="frameSearchOpen"
                  :overlay-store="appOverlayStore"
                  :recent="frameRecent"
                  :search="frameSearch"
                  @close="frameSearchOpen = false"
                  @open="openFrameResult"
                />
              </template>
            </SidebarLayout>
          </div>
          </GalleryOverlayWell>
        </GallerySpecimen>
        <div class="flex gap-2">
          <Button size="sm" @click="panelWork.reset">Reset Panel</Button>
          <!-- What the agent's `demi browser open` does, with `--show` or without: the browser tells the plugin of the new tab. -->
          <Button size="sm" @click="agentOpens(true)">Open and Show a Page as the Agent</Button>
          <Button size="sm" @click="agentOpens(false)">Open a Page to Check as the Agent</Button>
          <span class="self-center font-mono text-[11px] text-fg-faint">panel {{ Math.round(panelAsideShare * 100) }}% of the width it splits with the session · at least {{ ASIDE_SHARE.minWidth }}px, leaving the session {{ ASIDE_SHARE.mainMinWidth }}px</span>
        </div>
      </GallerySection>
      <GallerySection
        title="Work Panel"
        note="Changes and Files are pinned tabs, the `change` and `file` kinds of the changes and file-browser plugins: tabs of one shape whatever is selected, each its icon and its own short title, Changes with its counts after it, and Files never the open file’s name, which is in its tooltip, that lead the strip’s one tab list, so the arrow keys move across them and the user’s tabs alike; they are never created, closed or saved, never scroll or drag with the strip’s tabs, and only compete with them for the selection. A divider parts them from the user’s tabs. The strip holds the user’s tabs, content-sized, capped at 160px and an equal share of the strip, narrowing to 88px before it scrolls, with a button at its end that lists every tab once it scrolls, and close menus that affect only tabs. The strip has no control to open a tab: a tab appears when the agent opens one, or when a page in a tab opens one, as the first specimen’s Open and Show a Page as the Agent does. Before the browser has a tab, as while it reopens a tab it lost, the content says what Demi does from the frame the tab starts opening, centred with a spinner: Connecting… until the page has read what the Host is doing, then Starting Cloud…, Starting the browser… or Opening the page…, as the specimens held in each phase show, each a tab the browser lost that reopens; Stop the Cloud shows them in turn. An address typed before then shows at once with a thin loading line, and the tab opens there. Reload, Back, Forward and an address show the loading line on the click, before anything left the page: the gallery’s browser answers about a second later and starts loading a moment after its answer, as a far host does. The host numbers its tab lists and the answer names the last one before the request, so only a list numbered higher ends the line, whichever reaches the page first; a page that loads keeps the picture it had under the line until the browser says it stopped loading. Back and Forward are unavailable while the tab’s history has no page that way; go to another address to make Back available. A request the browser refuses ends the line at once and is reported in a toast, never in the tab; a tab the plugin could not open shows a region status with Retry. What the Host’s view cannot do for the viewer while the picture still shows, such as delivering its input, is a toast once; a Host that cannot capture its tabs shows a region status in place of the picture, as the specimen of one does. A closed tab leaves at once and never comes back, fading out as an unselected tab while the selection passes on. While a tab loads, it shows so in three places at once, as a web browser does: Reload turns into Stop, the tab’s mark in the strip turns into a spinner, and the line runs; Stop asks the browser to stop the page, and the three end once the browser says it stopped and, for a tab that shows no picture yet, such as one opening or reopening, once its first picture arrives. Stop is never unavailable while a tab loads: pressed before the browser has the tab, it gives up the address, and the tab opens blank. The gallery’s pages take five seconds to load, so Stop can be tried. A page the agent closed leaves the strip, as a closed tab does. A tab the browser lost when it ended, as a release or a stopped Cloud ends it, keeps its title and address, and opens its page again as soon as it is shown, loading as any page does; a reopening the host refuses shows its message with Retry, which returns the tab to loading at once. A view that reconnects keeps its last picture under a quiet note. A viewer’s browser that cannot decode the host’s H.264, such as a Chromium built without proprietary codecs, opens no view: its browser tab says so in place of the picture, as the last specimen’s does in any browser. A page holds no view while it is hidden, behind another browser tab or in a minimized window, and opens a new one when it is shown again: switch away from the gallery and back, and the picture connects again, its moving mark starting over. A Web picture stands at its own size from the panel’s top-left corner and is never scaled: while the panel grows, the part it does not cover yet is white until a picture at the new size arrives, and while it shrinks, the panel cuts it; a phone’s picture is scaled to fit and centred. The cursor over the page is the page’s own, resolved here from where the page says each cursor applies, so moving across the button shows its pointer at once; no caret or local control shows over the picture. A tab selected again after another tab, or after File, shows its picture, address and title at once, with no loading line for a page that had loaded, and the File view’s tree shows as it was, with no slide. The agent shows a tab with demi browser show, or opens one with --show: once its job ends, the panel opens, closed or not, and selects that tab, once per showing, and the user’s own selection after it stands; a tab the agent opens only to check its own work joins the strip without taking the view (the app frame’s controls show both). The viewport control is a square icon button, a computer or a phone, as far from the address as the navigation group is; its menu rows carry the same icons. Change groups its added/removed counts with a 2px gap and shows uncommitted totals, none while nothing changed; picked again, it shows the view as it was left, its mode and file included; file pills still open retained edits there. A browser tab wears its page’s icon, as a web browser’s tab does, and the globe only for a page without one, such as the Docs tab’s site. Its menu offers Reload before the Close commands. File uses a Lucide outline icon until a file is selected, then its file-type icon. Every tab content puts its address row immediately below the strip; the same divider as File and Change separates it from the page. Browser and File navigation buttons have no extra gap between them; both address bars leave 12px after the navigation group. A click into an address selects it whole, a second click places the caret, and Enter submits it and lets the field go, so keys reach the page again. The address bar reads what is typed as a browser’s does: localhost:3000 opens over http, example.com over https, words are a Google search, and Escape gives back the page’s address. With the focus in the tab, ⌘L focuses the address, ⌘[ and ⌘] go Back and Forward and ⌘R reloads the page, never Demi. The drawn page’s link opens the invoice in a new tab, which joins the strip at once and is selected, as a click’s new tab is. A right click on the page opens the browser’s menu: Back, Forward and Reload, a link’s Copy Link Address, Cut, Copy and Paste in the field; Escape closes it, as it closes every menu. A tab the page’s link opens goes right after its page’s tab, behind the tabs that tab opened before, as Chrome places them, and keeps that order after a reload. Ship order asks in a dialog titled with the page’s host, which Enter and Escape answer. Download guide.pdf shows the download in a bubble under the address bar’s download button: its size as it comes, then Save, which saves it to this computer, and Show in Files, which opens it in File. Beside a new conversation, before its first message, the panel binds no plugin and says that files and changes appear after the first message; Close is its only control."
      >
        <div class="grid gap-6 md:grid-cols-2">
          <GallerySpecimen variant="tabs" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="exhibitWork" @close="exhibitWork.reset" />
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="exhibitAgentOpens">Open and Show a Page as the Agent</Button>
              <span>A tab appears only when the agent or a page opens one.</span>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="tabs · the conversation browser’s tabs · live" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="browserWork" @close="productWould('The work panel closes.')" />
            </div>
            <!-- What the agent's close, and a conversation browser that ended, do to the tab being shown. -->
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" :disabled="shownBrowserTab === null" @click="closeOnDevice">Close the Page as the Agent</Button>
              <span>The tab leaves the strip, as a closed tab does.</span>
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="endBrowser">End the Browser on the Device</Button>
              <span>The shown tab opens its page again at once; the others when shown.</span>
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="stopCloud">Stop the Cloud</Button>
              <span>The shown tab opens again, saying what starts: the Cloud, then the browser.</span>
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="failNextOpen">Refuse the Next Opening on the Device</Button>
              <span>Then end the browser: the reopening fails, and Retry loads again.</span>
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="failNextStart">Fail to Start the Browser</Button>
              <span>Then end the browser: the reopening says so.</span>
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="failNextOnServer">Fail on the Server</Button>
              <span>Then end the browser: the reopening says so.</span>
            </div>
            <!-- What the agent's `demi browser show` does once its job ends. -->
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" :disabled="toShow === null" @click="agentShows">Show Another Tab as the Agent</Button>
              <span>The panel selects the tab the agent shows, once per showing.</span>
            </div>
            <!-- A notice the picture still shows through, as the Host sends one for input it could not deliver. -->
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="failInput">Fail the Viewer’s Input on the Host</Button>
              <span>A toast says so, once; the picture stays.</span>
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" @click="stopCapture">Stop Capturing on the Host</Button>
              <span>The browser tab says the device couldn’t capture it; Retry brings the picture back.</span>
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            v-for="phase in STARTING_PHASES"
            :key="phase"
            :variant="`tabs · before the browser has the tab · ${startingLabel(phase)}`"
            wide
          >
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="heldWorks[phase]" @close="productWould('The work panel closes.')" />
            </div>
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" :disabled="heldWorks[phase].browser.held.value === null" @click="finishStarting(phase)">Let the Host Go On</Button>
              <Button size="sm" @click="holdAgain(phase)">Show Another Lost Tab That Waits Here</Button>
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="tabs · a viewer’s browser that cannot decode H.264 · live" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="undecodedWork" @close="productWould('The work panel closes.')" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="tabs · a Host that cannot capture · live" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="uncapturedWork" @close="productWould('The work panel closes.')" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="before the first message · nothing to show yet" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="unstartedWork" before-first-message @close="productWould('The work panel closes.')" />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="Simulators"
        note="The conversation’s simulators in the work panel, the mobile plugin’s `mobile` kind (`packages/plugin-mobile/src/index.ts`). The panel opens no device of its own: Open a Simulator as the Agent opens one as demi mobile open does, an iPhone first, then an iPad, then an Android emulator, and its tab joins the strip. A device’s tab and its toolbar, before the name, show its system’s logo, drawn as a device’s mark draws one: Apple for the iPhone and the iPad, the Android robot’s head for the Android emulator. A device’s tab shows its toolbar: the device’s name where the whole name fits, left out where it does not and on a phone, whose tab names the device; Rotate Left, Rotate Right and Toggle Appearance turn and recolour the screen; Home and Lock press the buttons, and on the Android emulator Back, which leaves Settings; Volume Up and Down press theirs, which the gallery’s screen does not draw, so a toast says what the simulator would show; Screenshot and Record save on this computer, as a download does. Record shows Stop Recording while the device records, whoever started it: Record as the Agent starts or stops the agent’s recording of the first device, and the toolbar follows. Each control runs the operation of the command that does the same. The picture sits in the device’s frame; this Host has neither Xcode’s frames nor Android Studio’s device art, so the view draws the device itself in the look of Xcode’s frames: a black body with an even bezel concentric with the screen’s corners, the Android emulator’s the smooth curve its system UI draws, a metal edge, and its buttons as short bars, which turns with the device. A click on the screen is a tap, a drag a swipe; tapping the home screen’s upper half opens Settings, Home returns, and the scroll wheel scrolls Settings, as a swipe of its turn would on a device. On a panel too narrow for every control, the controls at the end go into the More menu (») at the toolbar’s end, as a macOS window’s toolbar puts them; narrow the window to see it. While the agent uses the device, from its first command until three seconds after its last, the view says The agent is using this device, and rings where it tapped; the panel’s own controls are the user’s and show neither. The controls that only send input, Home, Lock, the volume keys, Back, the rotations and Toggle Appearance, act on the press with no spinner. Lock turns the screen off: the picture keeps its last frame dimmed under Screen Off with Wake, and Wake, a tap or a key turns it on again; Sleep as the Device turns each screen off as a device’s own sleep does. Closing a device’s tab closes the device."
      >
        <GallerySpecimen variant="tabs · the conversation’s simulators · live" wide>
          <div class="gallery-frame flex h-[34rem] overflow-hidden">
            <GalleryWorkPanel class="w-full" :work="mobileWork" @close="productWould('The work panel closes.')" />
          </div>
          <div class="mt-2 flex flex-wrap items-center gap-2 text-[12px] text-fg-muted">
            <Button size="sm" @click="mobileWork.mobile.agentOpens()">Open a Simulator as the Agent</Button>
            <Button size="sm" @click="mobileWork.mobile.agentTaps()">Tap as the Agent</Button>
            <Button size="sm" @click="mobileWork.mobile.agentRecords()">Record as the Agent</Button>
            <Button size="sm" @click="mobileWork.mobile.devicesSleep()">Sleep as the Device</Button>
            <span>The agent’s tap shows its note and ring on the device it uses, the note for three seconds after.</span>
          </div>
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="File View"
        note="A file of the Host: its whole path as crumbs from the Host’s root, the root and the home wearing their glyphs, the file itself under its bar, and the workspace tree beside it following the file, as Visual Studio Code’s Explorer follows the file being edited: whenever the view shows a file, by a button here, the tree, the crumbs, Back or Forward, the tree unfolds the folders above it in turn as each lists, selects its row and scrolls it into view if it is not, the other folders staying as they are; packages/agent/src/providers/anthropic.ts is four folders down. A file outside the workspace, as Open a File Outside the Workspace opens one of another checkout, leaves no row selected. A hidden tree stays hidden and shows the file once its control shows it. The file’s bar, over the file and not the tree, says what the file is: the language’s full name as the editor knows it, or Plain Text, with its lines and size; an image’s kind, pixel size, zoom and size, the zoom following the picture; a video’s pixel size and size; another file’s kind and size. On its right are Preview and Source for Markdown and SVG, Copy, which puts the whole text on the clipboard, Markdown’s source and not its rendering, and turns to a check and “Copied” for a moment, and Download for every file. Text opens read-only in the code editor, colored by its language, with folding; a click puts a blinking caret in it, which the arrows move and Shift extends a selection from. Mod-f in the text opens a find bar below it: the query with its match count, “Match case”, “Match whole word” and “Use regular expression”, and “Previous match” and “Next match”, which Shift+Enter and Enter in the field also do. Typing selects the first match from the selection on, the count shows a question mark while the selection is on no match, a count past 9999 stops there with a +, and the scrollbar marks every match counted until Escape or Close shuts the bar. An image opens fitted to the pane without being enlarged, over a checkerboard where it is transparent, and zooms and moves as in the media viewer, by the same component: the wheel or a pinch zooms about the pointer up to 800%, a drag moves it while it is larger than the pane, and a click without a drag toggles the fitted and the actual size about the clicked spot; its bar says its pixel size, its zoom and its file size. Video and audio play in the browser’s own player and PDF in its own viewer, its bar saying its pixel size, for video, and its file size. Markdown renders like a repository file on GitHub: its HTML sanitized, so the script and the handler at the end of the README never run, its centered logo at its tag’s height in the lockup for the theme Demi shows, its badges on one line, its alerts titled Note and Warning, its math and code rendered, its front matter a YAML block, and its links opening files here, scrolling to headings, or leaving for the web. Markdown and SVG switch between Preview and Source. A file that is neither text nor previewable is a card with its facts and Download, and every file has Download in its bar. A crumb opens a menu of what lies beside it, directories unfolding into their own; a file picked there, clicked in the tree or linked from a document replaces the one shown, and Back and Forward before the crumbs walk the files shown. A click on the crumb row anywhere but a crumb turns it into a text field with the absolute path; a relative path starts from the workspace and ~ names the home, so ~/Projects/shop/README.md opens the README; the field completes from its folders and files as it is typed, a completed file opening at once: Enter opens a file, or goes to a folder as Finder’s Go to Folder does, unfolding it in the tree with the folders above it and selecting it until another file opens; a folder outside the workspace says the tree shows the workspace only. A click on a row gives the tree the keyboard: the arrows, Home, End and Enter move through it and act, and typing a name moves to the first row shown that starts with it, opening nothing. A right-click in the tree downloads a file, or uploads into a folder, or into the workspace from the empty space; the uploads list under the tree, moving here at a pace slow enough to watch. The control at the end of the crumb row hides and shows the tree; it grows from the end as the file gives way, and shrinks back, and a tree hidden and shown again is as it was left. A view that would keep less than 320px beside the tree hides it by itself; the control then slides the tree in over the file, which stays as it is, its bar’s controls moving to the tree’s side, and the control, a click beside the tree or a file picked in it slides it away. The tree docks again once the view is wide enough, and its divider stops where the file would get narrower than that; the narrow frame resizes from its corner. Reads carry the fixture’s latency, so the text and each directory show their loading state the first time; the fixture’s Host watches its files, as the product’s does, so a file or folder shown again shows at once, scrolled where it was, and asks nothing. Edit cookie.ts Outside Demi writes the file as another program would: the Host reports it, and the text is read again and replaced in place, without a loading state, the scroll and folds staying. A read again that fails keeps the text and says so quietly over it, in the page’s own words, with Retry, and the view reads it again by itself once the Host is back and its watch live again. A file that is not text keeps its card while it is read again. A file that is not there says it does not exist, its bar naming the file alone with nothing to do with it, with Retry, which shows the read on its way and then the same answer while the file is still not there. An image a document names that does not load shows its alt text in a dashed box, as the README’s missing one does, and tries again when the document is read again. server.log is over the text route’s 8 MiB: it opens as its first 8 MiB, with a line saying how much of how much shows and Download; its bar says its whole size and offers no Copy, since the page holds only the start. A file the Host cannot watch has the view say that it shows it as last read, with Refresh, and it is checked each time it shows."
      >
        <div class="flex flex-wrap gap-1">
          <Button v-for="file in previewFiles" :key="file" size="sm" @click="showInFileView(`${workspace.root}/${file}`)">{{ file }}</Button>
          <Button size="sm" @click="showInFileView(`${SLOT_ROOT}/README.md`)">Open a File Outside the Workspace</Button>
          <Button size="sm" variant="ghost" @click="editCookieOutside">Edit cookie.ts Outside Demi</Button>
        </div>
        <GallerySpecimen
          v-for="specimen in [
            { variant: 'workspace file · live', narrow: false },
            { variant: 'narrow · the tree hides, its control shows it over the file', narrow: true },
          ]"
          :key="specimen.variant"
          :variant="specimen.variant"
          wide
        >
          <div
            class="gallery-frame flex overflow-hidden"
            :class="specimen.narrow ? 'h-[32rem] max-w-full resize-x' : 'h-[40rem]'"
            :style="specimen.narrow ? { width: '30rem', minWidth: '16rem' } : undefined"
          >
            <FileView
              class="w-full"
              v-model:tree="fileViewTree"
              v-model:mode="fileViewMode"
              :source="workspace.source"
              :root="workspace.root"
              :path="fileViewPath"
              :can-back="fileViewBack.length > 0"
              :can-forward="fileViewForward.length > 0"
              @open="showInFileView"
              @back="fileViewGoBack"
              @forward="fileViewGoForward"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="narrow · no file chosen yet, the note centred beside the tree shown over it" wide>
          <div class="gallery-frame flex h-[24rem] max-w-full resize-x overflow-hidden" style="width: 30rem; min-width: 16rem">
            <FileView
              class="w-full"
              v-model:tree="unchosenTree"
              :source="workspace.source"
              :root="workspace.root"
              :path="unchosenPath"
              :can-back="unchosenPath !== null"
              @open="unchosenPath = $event"
              @back="unchosenPath = null"
              @forward="productWould('Show the File After')"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="a refresh that failed · the text stays, Retry reads it again" wide>
          <div class="flex flex-col gap-2">
            <div class="flex flex-wrap gap-1">
              <Button size="sm" @click="failCookieRefresh">Fail the Next Refresh</Button>
            </div>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <FileView
                class="w-full"
                :source="failingWorkspace.source"
                :root="failingWorkspace.root"
                :path="cookiePath"
                @open="productWould('Open the File in This View')"
                @back="productWould('Show the File Before')"
                @forward="productWould('Show the File After')"
              />
            </div>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="a file that does not exist · Retry reads it again" wide>
          <div class="gallery-frame flex h-[24rem] overflow-hidden">
            <FileView
              class="w-full"
              :source="workspace.source"
              :root="workspace.root"
              :path="`${workspace.root}/TODO.md`"
              @open="productWould('Open the File in This View')"
              @back="productWould('Show the File Before')"
              @forward="productWould('Show the File After')"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="a Host that cannot watch · files as last read, Refresh" wide>
          <div class="gallery-frame flex h-[24rem] overflow-hidden">
            <FileView
              class="w-full"
              :source="unwatchedWorkspace.source"
              :root="unwatchedWorkspace.root"
              :path="cookiePath"
              @open="productWould('Open the File in This View')"
              @back="productWould('Show the File Before')"
              @forward="productWould('Show the File After')"
            />
          </div>
        </GallerySpecimen>
        <GalleryFindBar />
      </GallerySection>
      <GallerySection
        title="Code"
        note="Code shows in one set of colors wherever it shows: the File view’s source, the Change view’s diff, a code block in a message or a document, and a shell call’s script in its row and its terminal. One highlighter reads it, the code editor’s, in One Dark Pro or One Light by the page’s mode, so the Rust and the Bash the reply quotes are the same colors as in their files beside it; switch the gallery’s mode to compare both. A fence language the highlighter does not know shows as plain text in the theme’s text color. A click in the File view’s source or the Change view’s diff puts a blinking caret there, in the theme’s caret color; the arrows move it, Shift extends the selection from it and Mod-f finds from it, and without the focus no caret shows."
      >
        <div class="flex flex-wrap gap-1">
          <Button v-for="file in quotedFiles" :key="file" size="sm" @click="quotedPath = file">{{ file.slice(workspace.root.length + 1) }}</Button>
        </div>
        <GallerySpecimen variant="a reply’s code beside its file · the same colors" wide>
          <div class="grid h-[40rem] grid-cols-2 gap-4">
            <div class="gallery-frame overflow-y-auto bg-surface px-6 py-5">
              <StreamedMarkdown class="text-conversation text-fg-body" :content="quotedReply" />
            </div>
            <div class="gallery-frame flex overflow-hidden">
              <FileView
                class="w-full"
                v-model:tree="quotedTree"
                :source="workspace.source"
                :root="workspace.root"
                :path="quotedPath"
                @open="quotedPath = $event"
                @back="productWould('Show the File Before')"
                @forward="productWould('Show the File After')"
              />
            </div>
          </div>
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="Change View"
        note="Diffs from one of two sources, the switch in the header picks. A click in a diff puts a blinking caret there, in the theme’s caret color: the arrows move it, Shift extends the selection from it, and Mod-f opens the find bar, which finds from it. A changed image, video, audio file or PDF shows its two versions side by side instead, each with its sizes, a new file only the second; a binary file with no preview shows a card per side with Download, and a committed version over 8 MiB says it is too large, with no Download. Markdown and SVG switch between the text diff and Preview, which renders both sides, labeled “Committed” and “Working tree” for the uncommitted changes, or “Before” and “After” for a commit and in Conversation. Commits shows the repository as VS Code’s Source Control and its commit list do together: the sidebar is two panes with a divider the user drags, kept as the sidebar’s width is; above, the selected item’s files as one tree, below, the commit list. The list shows one branch’s history, newest first, following first parents, each commit its summary, author and age, its whole message, author, time and id in its tooltip; more load as it scrolls to its end (main holds 64). While the list shows the branch checked out it starts with Uncommitted Changes and its count of files; another branch’s list starts with its newest commit. The branch menu left of the toolbar’s buttons names the branch the list shows and lists the local branches, most recently committed first, the checked-out one marked Checked Out; choosing one only changes the list. Selecting an item shows its files above and opens the first; a commit shows its changes against its first parent, each file marked A, M, D or R. Commit in a Terminal makes a commit on main, and every list showing main shows it at once, as the product’s watch of the repository does. A link to a commit, commit:3f2a9c1 in a message, opens Commits on it, its list on the branch shown when that has it, else the one checked out, else the most recent one that has it; a commit no branch holds shows alone, the list saying “This commit is on no branch.”; one the repository lacks says “No commit deadbee in ~/shop.” Uncommitted Changes is the working tree against the last commit: each file ending its row with its line counts and the letter VS Code’s Git marks it with, by VS Code’s own rules from git’s two status letters: “U” untracked, “A” added, “M” modified, “D” deleted (its name struck through), “R” renamed, “T” type changed, and an exclamation mark in conflict, in VS Code’s colors and with VS Code’s words as the tooltip; where git has a letter for both the index and the working tree, the working tree’s shows, so a staged new file edited again is M. The fixtures hold every mark. Conversation shows one request: the files it changed within its agent’s working directory, each ending its row with All Changes’ line counts when they were kept; a removed file is struck through and a renamed one’s tooltip names where it came from. A file shows All Changes, from before the request’s first edit to after its last, when both were kept, and otherwise opens at its first edit with contents; a file edited more than once steps through its edits, Edit 2 / 3, each named by its call’s title. A file without contents has no diff; a picture or other binary file says Demi keeps only the text of a conversation’s changes, with Open File; and a request an edit removed says it is no longer in the conversation. Back and Forward walk what the view has shown, items and files alike. The header also opens the selected file itself. Either sidebar has a toggle, and in a narrow view it hides and shows over the diff the way the File view’s tree does; the files’ caption lists the uncommitted changes again, its control turning while the list is on its way. A header over the diff names the file shown, in either mode, as GitHub’s diff header does: its icon, its folder from the workspace, its name, where a renamed file came from, and the line counts of the diff under it; a request’s files list no counts of their own. A request’s button opens Conversation on its first file’s All Changes, a file pill on its file at that call’s first edit; the Change tab picked again shows the view as it was left; with nothing picked, Conversation says how to fill it. A workspace outside a Git repository shows no sidebar or toggle in Commits, only “Not a Git repository”, with no Retry, a commit a link opens there as well. It keeps the last list when a listing failed, and says under the rows when the list was cut short. A host can name the workspace in place of its directory’s name, as the product does for the Cloud’s own session directory. The tree takes the keyboard as the File View’s does: typing a name moves to the first row shown that starts with it, and Enter selects a file; the commit list moves its selection with the arrows."
      >
        <div class="flex flex-wrap gap-1">
          <Button size="sm" variant="ghost" @click="commitInTerminal">Commit in a Terminal</Button>
        </div>
        <GallerySpecimen
          v-for="specimen in [
            { variant: 'commits · the branch checked out, Uncommitted Changes first', work: changeUncommitted, rootName: undefined, narrow: false },
            { variant: 'commits · another branch, its newest commit picked', work: changeOtherBranch, rootName: undefined, narrow: false },
            { variant: 'commits · a commit picked', work: changeCommit, rootName: undefined, narrow: false },
            { variant: 'commits · a link to a commit on no branch', work: changeNoBranch, rootName: undefined, narrow: false },
            { variant: 'commits · a link to a commit the repository lacks', work: changeMissingCommit, rootName: '~/shop', narrow: false },
            { variant: 'commits · narrow, the tree hides, its control shows it over the diff', work: changeUncommitted, rootName: undefined, narrow: true },
            { variant: 'commits · not a repository, named Workspace', work: changeNoRepository, rootName: 'Workspace', narrow: false },
            { variant: 'commits · a commit link opened outside a repository', work: changeNoRepositoryCommit, rootName: undefined, narrow: false },
            { variant: 'commits · listing failed, cut short', work: changeStale, rootName: undefined, narrow: false },
            { variant: 'conversation · a request, All Changes of its first file', work: changeRequest, rootName: undefined, narrow: false },
            { variant: 'conversation · a pill’s edit, the third of three', work: changeRequestEdit, rootName: undefined, narrow: false },
            { variant: 'conversation · a Markdown file picked', work: changeDocument, rootName: undefined, narrow: false },
            { variant: 'conversation · a removed file whose contents were not kept', work: changeRemoved, rootName: undefined, narrow: false },
            { variant: 'conversation · a picture and a packed release, which keep no contents', work: changeBinary, rootName: undefined, narrow: false },
            { variant: 'conversation · nothing picked', work: changeEmpty, rootName: undefined, narrow: false },
          ]"
          :key="specimen.variant"
          :variant="specimen.variant"
          wide
        >
          <div
            class="gallery-frame flex overflow-hidden"
            :class="specimen.narrow ? 'h-[32rem] max-w-full resize-x' : 'h-[40rem]'"
            :style="specimen.narrow ? { width: '30rem', minWidth: '16rem' } : undefined"
          >
            <ChangeView
              class="w-full"
              v-model:tree="changeViewTree"
              v-model:presentation="changeViewPresentation"
              :contents="workspace.source.contents"
              :mode="specimen.work.tab.value.mode"
              :selected="specimen.work.selected.value"
              :commit="specimen.work.tab.value.commit"
              :branch="specimen.work.tab.value.branch"
              :place="specimen.work.tab.value.place"
              v-model:commits-height="changeViewCommitsHeight"
              :edit="specimen.work.edit.value"
              :changes="specimen.work.changes.value"
              :root="workspace.root"
              :root-name="specimen.rootName"
              :can-back="specimen.work.tab.value.back.length > 0"
              :can-forward="specimen.work.tab.value.forward.length > 0"
              @update:mode="specimen.work.setMode"
              @update:selected="specimen.work.select"
              @update:commit="specimen.work.setCommit"
              @update:branch="specimen.work.setBranch"
              @placed="specimen.work.placed"
              @update:edit="specimen.work.setEdit"
              @back="specimen.work.back"
              @forward="specimen.work.forward"
              opens
              @open="(path: string) => productWould(`Open ${path} in File`)"
            />
          </div>
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'session'">
      <ChatSession @show-subagents="() => {}"
        :conversation="session"
        has-provider
        :share="shareService"
        :shares="sessionShares"
        :select-edit="(selection) => { panelWork.selectEdit(selection); view = 'panel' }"
        :files="sessionFiles"
        :edit-version="editVersion"
        :permission-requests="sessionRequests"
        @decide-permission="(id, decision) => (sessionRequests = decidePermission(sessionRequests, id, decision))"
        v-model:message-edit="messageEdit"
        @regenerate="regenerateMessage(editHost, $event)"
        :pending-submissions="sessionFlow.pendingSubmissions.value"
        :removing="sessionFlow.removing.value"
        @retry-undelivered="sessionFlow.retrySubmissions()"
        @edit-submission="sessionFlow.editSubmission"
        @delete-submission="sessionFlow.deleteSubmission"
        @retry="sessionFlow.resume()"
        @rename="session.title = $event"
        @save-scroll="(_id, state) => (session.scroll = state)"
        :reveal="sessionReveal"
        @revealed="sessionReveal = null"
        @reveal="(at) => (sessionReveal = at)"
        @abort-subagents="sessionFlow.abortSubagents"
        @abort-subagent="sessionFlow.abortSubagent"
        @abort-terminal="sessionFlow.abortTerminal"
        @recall-queued="sessionFlow.recallQueued"
        @send-queued="sessionFlow.sendQueued"
        @recall-steer="sessionFlow.recallSteer"
        @interrupt-pending-steer="sessionFlow.interruptPendingSteer"
        v-bind="agentContext.session"
      >
        <template #composer>
          <GalleryComposer
            v-model:message-edit="messageEdit"
            @submit-edit="submitEdit"
            placeholder="Ask Demi about the failing login test…"
            :running="session.phase === 'running' || sessionFlow.startingTurn.value"
            :compacting="session.phase === 'compacting'"
            :usage="session.contextUsage ?? undefined"
            :archived="session.archived"
            :flow="sessionFlow"
            @restore="session.archived = false"
            @send="sessionFlow.turn"
            @steer="sessionFlow.steer"
            @queue="sessionFlow.queue"
            @stop="sessionFlow.stop"
            @compact="sessionFlow.compact"
          />
        </template>
      </ChatSession>
    </template>
  </div>
</template>

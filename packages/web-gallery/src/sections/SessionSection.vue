<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { Play } from '@lucide/vue'
import ThinkingBlock from '@demicodes/web-ui/agent/blocks/ThinkingBlock.vue'
import AgentReceiptBlock from '@demicodes/web-ui/agent/blocks/AgentReceiptBlock.vue'
import { agentReceiptMessages, editedFile, permissionReceiptMessages } from '../fixtures/blocks'
import PermissionCard from '@demicodes/web-ui/permissions/PermissionCard.vue'
import { afterDecision, type PermissionDecision, type PermissionRequestView } from '@demicodes/web-ui/permissions/types'
import { queuedRequests, rootRequest, subagentRequest } from '../fixtures/permissions'
import { readGalleryEdit } from '../fixtures/blobs'
import ErrorBlock from '@demicodes/web-ui/agent/blocks/ErrorBlock.vue'
import ToolShellBlock from '@demicodes/web-ui/agent/blocks/ToolShellBlock.vue'
import ToolCallBlock from '@demicodes/web-ui/agent/blocks/ToolCallBlock.vue'
import { parseToolInput } from '@demicodes/web-ui/agent/block-helpers'
import ActivitySlot from '@demicodes/web-ui/agent/blocks/ActivitySlot.vue'
import type { ActivityKind, HandoffBlock } from '@demicodes/web-ui/agent/activity-slot'
import { provideEditSelection } from '@demicodes/web-ui/agent/edit-selection'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import GalleryWorkPanel from '../components/GalleryWorkPanel.vue'
import { galleryFiles, useGalleryWork } from '../fixtures/work-panel'
import { changePath, firstChangeData, goBack as changeBack, goForward as changeForward, showChange } from '@demicodes/plugin-changes/data'
import { selectedTab } from '@demicodes/web-ui/agent/panel-tabs'
import type { PanelTabKind } from '@demicodes/web-ui/agent/panel-kinds/kind'
import { browserTabDataSchema } from '@demicodes/plugin-browser/live/tabs'
import { callChangeSource, type CallEditSelection, type ChangeMode, type ChangeSources } from '@demicodes/web-ui/files/changes'
import ChangeView from '@demicodes/web-ui/files/ChangeView.vue'
import FileView from '@demicodes/web-ui/files/FileView.vue'
import { createGalleryChangeSet, createGalleryWorkspace, gallerySides } from '../fixtures/workspace'
import { galleryBrowser, type GalleryBrowser } from '../fixtures/live-browser'
import { galleryConversationFiles } from '../fixtures/message-files'
import GalleryAttachmentMessage from '../components/GalleryAttachmentMessage.vue'
import { productWould } from '../product-would'
import { sidebarEntries } from '@demicodes/web-ui/plugins/page'
import { PLUGIN_PAGES } from '../generated/pages'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import { ASIDE_SHARE, SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
import { demoAccount, demoConversations, demoProjects } from '../sidebar/sidebar-data'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import UserBlock from '@demicodes/web-ui/agent/blocks/UserBlock.vue'
import ModelMenu from '@demicodes/web-ui/agent/ModelMenu.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
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
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import TerminalChip from '@demicodes/web-ui/agent/TerminalChip.vue'
import TerminalPanel from '@demicodes/web-ui/agent/TerminalPanel.vue'
import { runningSubagents } from '@demicodes/web-ui/agent/subagents'
import GalleryCachedSessions from '../components/GalleryCachedSessions.vue'
import GalleryTranscriptEntrance from '../components/GalleryTranscriptEntrance.vue'
import GalleryConnectedSession from '../components/GalleryConnectedSession.vue'
import GalleryMessageEditing from '../components/GalleryMessageEditing.vue'
import GalleryAssistantMessages from '../components/GalleryAssistantMessages.vue'
import GalleryModelPreference from '../components/GalleryModelPreference.vue'
import GalleryFindBar from '../components/GalleryFindBar.vue'
import GalleryUserMessageLengths from '../components/GalleryUserMessageLengths.vue'
import { submitMessageEdit, type MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import { callTerminal, firstRunningTerminalId } from '@demicodes/web-ui/agent/terminals'
import { provideLiveCalls } from '@demicodes/web-ui/agent/live-calls'
import type { UserContentBlock } from '@demicodes/protocol'
import { applyModelChange, type ModelSettings, type ModelSettingsChange } from '@demicodes/web-ui/agent/model-selection'
import { composerAttachment, encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Button from '@demicodes/web-ui/ui/Button.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import {
  demoImageUrl,
  demoModel,
  changesDemoBlocks,
  editingShellTool,
  fileChangeCases,
  fullPageTool,
  longScriptShellTool,
  missingImageTool,
  notStoredVideoTool,
  recordingTool,
  runningShellTool,
  screenshotTool,
  screenshotsTool,
  shellTool,
  smallImageTool,
  statusImageTool,
  thinkingText,
  steerPrompt,
  transcriptDemoBlocks,
  unsizedRecordingTool,
  wideImageTool,
} from '../fixtures/blocks'
import {
  demoModels,
  demoProviders,
  offlineProviders,
  openaiOnlyProviders,
  usageAt,
} from '../fixtures/catalog'
import { galleryBlobs } from '../fixtures/blobs'
import { gallerySubagents } from '../fixtures/subagents'
import { galleryTerminals } from '../fixtures/terminals'
import { demoDeviceStart } from '../fixtures/device-installation'
import { useLiveGalleryCommand } from '../live-command'
import { useTurnFlow, type TurnFlowKind } from '../turn-flow'
import { APP_SHORTCUTS } from '@demicodes/web-ui/settings/shortcuts'
import GalleryComposer from '../components/GalleryComposer.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GalleryContextLimit from '../components/GalleryContextLimit.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import GalleryTabStripDrive from '../components/GalleryTabStripDrive.vue'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()
const historyModelBlocks = transcriptDemoBlocks()

/**
 * The permission card's specimens, each over its own requests: a decision
 * drops what the product's would (an allow every request of its category, a
 * deny that one) and says what the product would tell the agent; Show Again
 * brings the requests back.
 */
const permissionSpecimens = reactive([
  { variant: 'one request', fixture: () => [rootRequest()], requests: [rootRequest()] },
  { variant: 'a queue of three', fixture: queuedRequests, requests: queuedRequests() },
  { variant: 'a subagent asked', fixture: () => [subagentRequest()], requests: [subagentRequest()] },
])
function decidePermission(
  requests: PermissionRequestView[],
  id: string,
  decision: PermissionDecision,
): PermissionRequestView[] {
  productWould(decision === 'allow'
    ? 'Allow the Category and Tell Each Agent That Asked'
    : 'Tell the Agent the Request Was Denied')
  return afterDecision(requests, id, decision)
}
/**
 * The dock's specimens: what stacks over the composer, one step apart, with
 * and without the card and the chips. A decision drops the card as the
 * product's would, and Show Again brings it back; a chip says what the
 * product would open.
 */
const dockSpecimens = reactive([
  { variant: 'card, no chips', asks: true, requests: [rootRequest()], chips: false },
  { variant: 'card over chips', asks: true, requests: [rootRequest()], chips: true },
  { variant: 'chips alone', asks: false, requests: [] as PermissionRequestView[], chips: true },
])
// The product session opens with the root's request waiting, above its chips.
const sessionRequests = ref<PermissionRequestView[]>([rootRequest()])
// A first message the server never confirmed. Retry sends it again with the
// same id; this time it arrives and its turn runs.
const deliveryFlow = useTurnFlow({ id: 'gallery-delivery' })
const undeliveredText = `Review the notes ${ATTACHMENT_MARK} before the next run.`
const undeliveredContent = joinMessageContent(undeliveredText, [[{
  type: 'attachment',
  name: 'notes.txt',
  path: '/home/demi/.demi/attachments/gallery/notes.txt',
  mediaType: 'text/plain',
  sizeBytes: 2048,
  sha256: '0'.repeat(64),
} satisfies UserContentBlock]])

function failDelivery(): void {
  deliveryFlow.undelivered(
    undeliveredContent,
    { text: undeliveredText, attachments: [composerAttachment({ name: 'notes.txt', phase: 'ready' })] },
    'Connection closed before confirmation',
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
const exhibitAgentId = ref<string | null>(
  runningSubagents(agents)[0]?.id ?? agents[0]?.id ?? null,
)
const exhibitTerminalId = ref<string | null>(firstRunningTerminalId(terminals))
const fillPane = computed(() => view.value === 'session')
const editVersion = computed(() => ({ epoch: 'gallery-session', revision: editRevision.value }))

watch(
  view,
  (next) => {
    if (next !== 'windows') {
      return
    }
    exhibitAgentId.value = runningSubagents(agents)[0]?.id ?? agents[0]?.id ?? null
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
const fileViewMode = ref<'preview' | 'source'>('preview')
const changeViewPresentation = ref<'diff' | 'preview'>('diff')
/** One of each kind the File view previews, by workspace path. */
const previewFiles = [
  'README.md', 'assets/logo.svg', 'assets/photo.png', 'assets/demo.mp4',
  'assets/tone.m4a', 'docs/guide.pdf', 'dist/app.zip', 'src/auth/cookie.ts', 'server.log',
]
/** One Change view on its own, stepped the way the `change` kind steps it: for the Change view specimens. */
function useChangeTab(mode: ChangeMode, path: string | null, changes: ChangeSources) {
  const tab = ref(showChange(firstChangeData(), mode, path, { call: changes.conversation, edit: 0 }))
  /** The file the view holds in its mode, else the first there is. */
  const selected = computed(() => changePath(tab.value, tab.value.mode, changes.uncommitted.files))
  function show(mode: ChangeMode, path: string | null) {
    tab.value = showChange(tab.value, mode, path)
  }
  return {
    tab,
    selected,
    changes: computed<ChangeSources>(() => ({
      uncommitted: changes.uncommitted,
      conversation: tab.value.call && changes.conversation
        ? { ...tab.value.call, read: changes.conversation.read }
        : null,
    })),
    setMode: (mode: ChangeMode) => show(mode, changePath(tab.value, mode)),
    select: (path: string | null) => show(tab.value.mode, path),
    back: () => (tab.value = changeBack(tab.value)),
    forward: () => (tab.value = changeForward(tab.value)),
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
const sessionFiles = galleryConversationFiles(workspace.source, (path) => {
  panelWork.openIn({ intent: 'file', payload: { path } })
  panelAsideOpen.value = true
  view.value = 'panel'
})
// The tabs specimen starts on an empty strip, with the globe-plus add control.
const exhibitWork = useWorkTabs('file', { browser: galleryBrowser([]) })
const editWork = useWorkTabs('change')
// The gallery's own conversation browser stands behind every specimen's `browser`
// kind, which lists its tabs as it is made.
provideEditSelection(() => editWork.selectEdit)
const changeUncommitted = useChangeTab('uncommitted', 'src/auth/cookie.ts', { uncommitted: workspace.changes, conversation: null })
const changePicked = useChangeTab('conversation', 'src/auth/cookie.ts', {
  uncommitted: workspace.changes,
  conversation: callChangeSource({
    commandId: 'gallery-cookie-edit',
    file: editedFile({ path: 'src/auth/cookie.ts', kind: 'modified', added: 1, removed: 1 }),
  }, readGalleryEdit),
})
const changeDocument = useChangeTab('conversation', 'README.md', {
  uncommitted: workspace.changes,
  conversation: callChangeSource({
    commandId: 'gallery-readme-edit',
    file: editedFile({ path: 'README.md', kind: 'modified', added: 0, removed: 1 }),
  }, () => gallerySides('README.md')),
})
const changeEmpty = useChangeTab('conversation', null, { conversation: null, uncommitted: workspace.changes })
const changeNoRepository = useChangeTab('uncommitted', null, {
  conversation: null,
  uncommitted: createGalleryChangeSet(200, { unavailable: 'no-repository' }),
})
const changeStale = useChangeTab('uncommitted', 'src/auth/cookie.ts', {
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
/** The device refuses the next tab it would open, as an offline device does: a reopening shows Retry. */
function failNextOpen() {
  browserWork.browser.failNextOpen()
  productWould('The Device Refuses the Next Tab It Opens')
}
// The same conversation browser, viewed in a web browser that cannot decode H.264, such as a Chromium without
// proprietary codecs.
const undecodedWork = useWorkTabs('change', { path: '', pictures: async () => false })
// A Host without the browser, which only the agent installs: the add control
// is disabled and says so, and the tabs already open stay.
const noBrowserWork = useWorkTabs('change', { path: '', browser: galleryBrowser(undefined, { chrome: false }) })
// A Host that cannot capture its tabs, such as an Apple M4's Linux VM: the agent's tab shows why it has no picture.
const uncapturedWork = useWorkTabs('browser-t1', { path: '', browser: galleryBrowser(undefined, { capture: false }) })
/** What the Host's live view module does when it cannot deliver the viewer's input: it tells the viewer in a notice. */
function failInput() {
  browserWork.browser.fail('input_failed', 'Input.dispatchKeyEvent: the target closed')
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
session.pendingSteers = [{ id: 'pending-1', content: steerPrompt }]
session.queue = [
  { id: 'q1', content: [{ type: 'text', text: 'Also add a case for the expired cookie.' }] },
  { id: 'q2', content: [{ type: 'text', text: 'Keep the light-mode screenshot in the same PR.' }] },
]
const streamFlow = useTurnFlow({ id: 'gallery-stream' })
const turnFlow = useTurnFlow({ id: 'gallery-turn' })
// The specimens' calls show their commands' output while they run: the live
// call's and the Turn's, whose flow runs its own command.
provideLiveCalls((toolUseId) =>
  callTerminal([...terminals, ...turnFlow.state.terminals], undefined, toolUseId),
)
const changesFlow = useTurnFlow({ id: 'gallery-changes', title: 'Cookie rename', blocks: changesDemoBlocks() })
const changesSurface = ref<{ dockHeight: number }>()
const changesList = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()
const turnSurface = ref<{ dockHeight: number }>()
const turnList = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()
const streamSurface = ref<{ dockHeight: number }>()
const streamList = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()

function playStream(): void {
  streamFlow.play('stream')
}

function playTurn(kind: TurnFlowKind): void {
  turnFlow.play(kind)
}

/** 150K tokens: half of the 300K the Gemini specimen is limited to. */
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
    path: '/home/demi/.demi/attachments/demo/login-failure.pdf',
    mediaType: 'application/pdf',
    sizeBytes: 120334,
    sha256: 'demo-pdf',
  },
  { type: 'text', text: ', the screenshot from CI is ' },
  { type: 'image', source: { type: 'url', url: demoImageUrl } },
  {
    type: 'attachment',
    name: 'login-fail.png',
    path: '/home/demi/.demi/attachments/demo/login-fail.png',
    mediaType: 'image/png',
    sizeBytes: 48211,
    sha256: 'demo-png',
  },
  { type: 'text', text: ' and its log is ' },
  {
    type: 'attachment',
    name: 'ci.log',
    path: '/home/demi/.demi/attachments/demo/ci.log',
    mediaType: 'text/plain',
    sizeBytes: 2048,
    sha256: 'demo-log',
    snippet: pastedSnippet,
  },
  { type: 'text', text: '. The manifest on my laptop is ' },
  {
    type: 'reference',
    reference: encodeRemoteReference('zan-mbp', '/Users/zan/Projects/demi/package.json'),
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
  uploading: `Wait for ${ATTACHMENT_MARK} and ${ATTACHMENT_MARK} to arrive.`,
  failed: `The capture ${ATTACHMENT_MARK} did not upload; the log ${ATTACHMENT_MARK} did.`,
  pasted: `Summarize ${ATTACHMENT_MARK}, then check it against ${ATTACHMENT_MARK}.`,
  long: 'The login test in packages/web/src/auth.test.ts is failing after the session cookie rename, and the fixture it reads has the old name.',
  formatted: [
    'The **modal** `padding` is off by *one* column, ~~not two~~. See [the layout notes](docs/layout.md) and https://example.com/issues/42.',
    '```ts',
    'export const padding = 12',
    '```',
  ].join('\n'),
}
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
const functionalCollapsed = ref(false)
const functionalExpanded = ref(true)
const functionalTool = ref(false)
const functionalShellExpanded = ref(true)
const functionalShellLive = ref(true)
const functionalShellScript = ref(true)
/** Calls whose results carry media, as the transcript shows them. */
const toolMediaSpecimens = [
  { variant: 'shell · image', block: screenshotTool },
  { variant: 'shell · several images', block: screenshotsTool },
  { variant: 'shell · tall image', block: fullPageTool },
  { variant: 'shell · wide image', block: wideImageTool },
  { variant: 'shell · small image', block: smallImageTool },
  { variant: 'shell · video', block: recordingTool },
  { variant: 'shell · video of unknown size', block: unsizedRecordingTool },
  { variant: 'status · image', block: statusImageTool },
  { variant: 'shell · video not stored', block: notStoredVideoTool },
  { variant: 'shell · image cannot load', block: missingImageTool },
]
const functionalShellFiles = ref(false)
const changeCaseOpen = reactive<Record<string, boolean>>({})
const functionalThinkingStartedAt = new Date().toISOString()
const functionalThinkingEndedAt = new Date(
  Date.parse(functionalThinkingStartedAt) + 8_000,
).toISOString()
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
const incomingThinking: HandoffBlock = {
  type: 'thinking',
  id: 'incoming-thinking',
  createdAt: functionalThinkingStartedAt,
  model: demoModel,
  text: '',
  signature: null,
}

async function submitEdit(): Promise<void> {
  await submitMessageEdit({
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
  })
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
})

onBeforeUnmount(() => {
  sessionRestore.stop()
  composerModelRestore.stop()
})
</script>

<template>
  <div :class="fillPane ? 'flex min-h-0 flex-1 flex-col' : 'flex flex-col gap-10'">
    <template v-if="view === 'tabs'">
      <GallerySection
        title="Motion"
        note="The work panel’s browser tabs, and every motion their strip owns, in a narrow frame that overflows and one at the pane width. Tabs fit their mark and title up to 160px; longer titles truncate. As in Chrome, a strip that runs out of room first shrinks its tabs, down to their mark and a few characters, and only then scrolls, so a tab opened in the background normally shows. A tab opens by growing from nothing and closes by collapsing, while the New browser tab control follows the last tab until they scroll and then holds the right edge. The selected tab always shows whole, its close control included, and never under a fade: selecting a tab, opening one, the agent showing one, the strip resizing and the selected tab’s title widening it each scroll, with the same timing as the tab, until it shows whole with a fade’s width beside it; where the strip has no room for that, the fades give way beside it, and no tab is wider than the strip has room for, so a very narrow strip truncates the selected tab’s title. Unselected tabs may lie under the fades, and so may the selected tab once the user scrolls it under an edge: the scrolled edge fades as the other does. Closing scrolls back when the end comes into reach. Wherever the strip comes to rest, a tab mark its edge cuts stays under the fade’s solid start, so a mark at a scrolled edge shows whole or not at all, never sliced. The crowded strip has the product’s panel width and five tabs, the last selected. Play all runs through the cases; the tabs, their close controls and their menus work too. The tabs act as a web browser’s: a middle click closes one, a drag moves one along the strip while the others step aside, and Escape puts it back; a focused tab moves the focus with the arrows, Home and End, and Enter or Space selects it. A tab’s tooltip comes only when the pointer moved onto it and rests there, never after a click, nor for a tab that slid under a pointer at rest, as a new one does under the New Tab control."
      >
        <GallerySpecimen variant="narrow · overflows" wide>
          <GalleryTabStripDrive width="32rem" />
        </GallerySpecimen>
        <GallerySpecimen variant="crowded · panel width, the last of five selected" wide>
          <GalleryTabStripDrive width="37.25rem" :pages="5" select="last" />
        </GallerySpecimen>
        <GallerySpecimen variant="wide · pane width" wide>
          <GalleryTabStripDrive />
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'composer'">
      <GallerySection
        title="Composer"
        note="Idle through Fast Mode; text formatted as it is typed; each file a capsule where it was put, through its upload phases, a failed upload with Retry, and a remote host file naming its device; and queue. Enter on a message that cannot go shows the send button’s reason where the pointer would. A message opens the composer as soon as it needs more than the line it is on, whether it holds lines of its own or its text outgrows the width, and closes it again when it fits; nothing is ever cut off at the line’s end. One send; a running turn queues, and Stop stays beside Queue for the whole turn. When a later save replaced a version of the draft, a row above the input offers it by its first line: Restore exchanges it with the draft, which the row then offers, and × dismisses it. When the user’s plugins changed since the conversation opened, a row above the input offers Reload, and goes once the conversation opened again. An unavailable last model keeps the chip, warns, and blocks send. No usable model and an archived conversation both replace the input with the same snackbar: a line on the left, Configure Models or Restore Conversation on the right. A conversation its Cloud holds during a reset uses that snackbar with a spinner and no action, and the input returns with its draft when the reset ends. The input keeps at least 128px: where the model chip’s name and level would leave it less, the chip is its sparkle alone, the name and level in its tooltip, and it names the model again once there is room; loading and a failed load give way the same. The narrow composer resizes from its corner."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen
            variant="idle · empty"
            wide
          >
            <GalleryComposer placeholder="Ask Demi…" />
          </GallerySpecimen>
          <GallerySpecimen
            variant="narrow · the model chip is its icon"
            wide
          >
            <!-- The frame keeps the dock's 12px around the composer, as the session does. -->
            <div class="max-w-full resize-x overflow-hidden p-3" style="width: 367px; min-width: 15.5rem">
              <GalleryComposer placeholder="Ask Demi…" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="focused"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              focused
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="send-ready"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              draft="The login test in packages/web/src/auth.test.ts is failing after the session cookie rename."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="a later save replaced a version · Restore exchanges them"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              draft="Fix the login test in packages/web/src/auth.test.ts."
              replaced="Fix the login bug before the release; the session cookie was renamed."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="primary Host offline · how to start its runner again, with Copy"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              :offline-host="{ name: 'zan-mbp', start: demoDeviceStart('macos') }"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="plugins changed since it opened · Reload opens it with the plugins on"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              plugins-changed
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="multiline"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              draft="The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.&#10;Keep the fix in that file."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="longer than its line"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              :draft="composerDrafts.long"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="formatting · as it will look"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              :draft="composerDrafts.formatted"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · capsules in the text"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
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
                  host: 'zan-mbp',
                  path: '/Users/zan/Projects/demi/package.json',
                },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="attachments · uploading"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
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
            variant="attachments · an upload failed · Retry"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
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
            variant="drop · where the files land"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
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
                placeholder="Ask Demi…"
                attach-open
              />
            </GalleryOverlayWell>
          </GallerySpecimen>
          <GallerySpecimen
            variant="queue · Stop stays beside it"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              running
              draft="Also add a case for the expired cookie."
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="stop"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              running
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="fast"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              service-tier-id="priority"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="no reasoning"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              selected-provider-id="openai"
              selected-model-id="gpt-5"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="context · warning"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              :usage="usageAt(0.75)"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="model unavailable"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
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
              placeholder="Ask Demi…"
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
              placeholder="Ask Demi…"
              :model-load="composerModelLoad"
              @retry-models="retryModels"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="held · Cloud is resetting"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
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
              placeholder="Ask Demi…"
              draft="Run the login test again."
              :hold="composerHeld ? 'Cloud is resetting.' : null"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="archived"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
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
              placeholder="Ask Demi…"
              :archived="composerArchived"
              @restore="composerArchived = false"
            />
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="PermissionCard"
        note="An agent’s command needs the user’s permission (permissions.md): the oldest request waits above the composer, below the transcript and over the dock’s chips, while the transcript and the composer stay usable. It says what the conversation would be allowed, the command the agent ran, the subagent that ran it, what a grant allows, and which one of how many it is. Allow for This Conversation decides every request of its category; Deny decides this one, and the next takes its place. A category the user’s command set no longer declares shows its id."
      >
        <div class="specimen-stack">
          <GallerySpecimen
            v-for="specimen in permissionSpecimens"
            :key="specimen.variant"
            :variant="specimen.variant"
            wide
          >
            <div class="w-full max-w-[44rem]">
              <PermissionCard
                v-if="specimen.requests.length"
                :requests="specimen.requests"
                @decide="(id, decision) => (specimen.requests = decidePermission(specimen.requests, id, decision))"
              />
              <div v-else class="flex items-center gap-3 text-chrome text-fg-subtle">
                Every request is decided.
                <Button size="sm" @click="specimen.requests = specimen.fixture()">Show Again</Button>
              </div>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>

      <GallerySection
        title="SessionDock"
        note="What waits over the composer stacks one 8px step apart: the permission card, then the chips, then the composer. A part that is not there takes no room, so without chips the card sits one step above the composer, as the chips do without a card. The scroll-to-bottom control floats over the transcript at the dock’s top right and takes no room either."
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
              <div class="rounded-lg bg-surface p-3">
                <SessionDock>
                  <template v-if="specimen.requests.length" #above>
                    <PermissionCard
                      :requests="specimen.requests"
                      @decide="(id, decision) => (specimen.requests = decidePermission(specimen.requests, id, decision))"
                    />
                  </template>
                  <template #chips>
                    <template v-if="specimen.chips">
                      <SessionDockChip @click="productWould('Resume the Turn')">
                        <Play :size="ICON_PX.in28" />
                        Resume
                      </SessionDockChip>
                      <AgentsChip :agents="agents" @open="productWould('Open the Agents Window')" />
                    </template>
                  </template>
                  <GalleryComposer placeholder="Ask Demi…" />
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
              :models="demoModels"
              :settings="selectorSettings"
              @change="changeSelector"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="fast">
            <ModelSelector
              :providers="demoProviders"
              :models="demoModels"
              :settings="fastSettings"
              @change="changeFast"
            />
          </GallerySpecimen>
          <GalleryOverlayWell size="wide">
            <GallerySpecimen variant="menu">
              <ModelMenu
                :providers="demoProviders"
                :models="demoModels"
                :settings="selectorSettings"
                @change="changeSelector"
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
        note="28px capsule with status dots. The active Agents dot breathes gently. Running and Agents open their windows; a second click closes them."
      >
        <div class="specimen-row">
          <GallerySpecimen variant="resume">
            <SessionDockChip>
              <Play :size="ICON_PX.in28" />
              Resume
            </SessionDockChip>
          </GallerySpecimen>
          <GallerySpecimen variant="running">
            <TerminalChip :terminals="terminals" />
          </GallerySpecimen>
          <GallerySpecimen variant="ready">
            <SessionDockChip dot="success">Ready</SessionDockChip>
          </GallerySpecimen>
          <GallerySpecimen variant="idle">
            <SessionDockChip dot="muted">Idle</SessionDockChip>
          </GallerySpecimen>
          <GallerySpecimen variant="agents">
            <AgentsChip :agents="agents" />
          </GallerySpecimen>
          <GallerySpecimen variant="agents · none running · hidden">
            <AgentsChip :agents="finishedOnly" />
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'blocks'">
      <GallerySection
        title="Assistant Message"
        note="Intermediate updates have no copy, fork or timestamp toolbar. Final replies keep their toolbar, including while a later user request runs. Preview fork success, pending creation, and a retry after failure."
      >
        <GallerySpecimen variant="message footer" wide>
          <GalleryAssistantMessages />
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="Attachments in a Message"
        note="What the agent gave the user with demi attachment upload, named as attachment:a3: an image shows and a click shows it large, a video shows its first frame with a play mark at an image’s bounds and a click plays it in the viewer, a link opens the attachment, an image or a video in the viewer and any other file as a download, and a number the conversation does not have says so. Images and videos that follow each other, with nothing but white space between them, stand side by side 8px apart and wrap: each is a thumbnail, as a tool’s are, the tall page cropped to its top and the wide timeline to its middle, whose box its record’s pixel size gives before its bytes arrive. A lone image after text, and a lone video’s first frame, keep the message’s width and 60% of the visible height; the demo’s record carries no size, so its frame is 16:9 until it arrives."
      >
        <GallerySpecimen variant="attachments" wide>
          <GalleryAttachmentMessage :files="sessionFiles" :cwd="workspace.root" />
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="UserBlock"
        note="User; files as capsules where they were put, a picture or a text file’s opening lines shown when pointed at; formatting; pending steer, queued, and stuck. Long messages have a view of their own: Lengths."
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
                @remove="pendingSteerShown = false"
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
                @remove="queuedShown = false"
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

      <GallerySection title="AgentReceiptBlock" note="Agent updates, completion receipts, and the user’s decisions on permission requests as the agent that asked received them. Expand to read the message; these rows have no human message controls.">
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack [--agent-pad-x:0px]">
            <GallerySpecimen
              v-for="(message, index) in [...agentReceiptMessages, ...permissionReceiptMessages]"
              :key="message.id"
              :variant="message.event.type === 'message' ? 'update' : message.event.outcome"
              wide
            >
              <AgentReceiptBlock :message="message" :open="index === 1" />
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="FunctionalBlock"
        note="Thinking, shell (collapsed and expanded), loading, and error. A shell call shows its command and output in one box: the command stays at the top and only the output under it scrolls once it fills the box. Command and output wrap as a terminal wraps: each line fills to the box’s edge and breaks at any character, never earlier at a hyphen, as the live output’s flags show. A command longer than two lines shows two, the second ending in an ellipsis: a click on it shows it whole and another click clamps it again; one that fits is no control, and a click selects it for copying. A whole command taller than half the box scrolls on its own. While a shell call runs, its command’s output shows under it as it comes, a line every frame here, and the box follows the newest line; scroll up to read, and it stays where you are until you scroll back to the end. Once the call returned, the call keeps the view its result stored."
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
              variant="shell · collapsed"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalTool"
                :block="shellTool"
                :input="parseToolInput(shellTool.input)"
                :is-streaming="false"
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
                :is-streaming="false"
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
                :is-streaming="false"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · long command"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalShellScript"
                :block="longScriptShellTool"
                :input="parseToolInput(longScriptShellTool.input)"
                :is-streaming="false"
              />
            </GallerySpecimen>
            <GallerySpecimen
              variant="shell · changed files"
              wide
            >
              <ToolShellBlock
                v-model:open="functionalShellFiles"
                :block="editingShellTool"
                :input="parseToolInput(editingShellTool.input)"
                :is-streaming="false"
              />
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
        title="Tool Media"
        note="The images and videos a call’s result carries show under its row, folded or open, side by side in the order of the result. Each is a thumbnail: 80px tall and 64 to 200px wide by its proportions, cropped beyond them (the tall page keeps its top, the wide timeline its middle), never enlarged (the icon keeps its size). Its reference carries its pixel size, so its box is final before its bytes arrive and nothing moves when they load; a video of unknown size is 16:9 until its first frame arrives. A video’s thumbnail is its first frame with a play mark; it never plays in place. A click opens the medium whole and large over the dimmed page: fitted and never enlarged; a click on an image toggles actual size, a video plays at once with the browser’s controls, whose full-screen control shows it larger, and closing stops it. Escape, the close control or a click on the dimmed page closes the viewer, and the picture stays until it has faded out. A medium that is gone shows one line where it was: that it was not stored, with the store’s reason, or the day it was removed. A medium the page cannot show says so at the same height. At a phone’s width a preview fits the column, an image opened large fills the screen, where a tap toggles actual size, and an iPhone plays a video full screen."
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
                :is-streaming="false"
              />
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="ActivitySlot"
        note="Requesting while the provider is asked, from the send of a message, its delivery included, and a Resume or Continue alike, so a slow model reads as the provider’s wait. The word and its clock stay through the server’s confirmation of the message and the agent’s own retries after a failed attempt: they are the same wait, and a row that changed its word with each attempt would flicker. Elapsed time shows after one second; incoming blocks roll into the same row."
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
          <GallerySpecimen
            variant="incoming · thinking"
            wide
          >
            <div class="gallery-frame gallery-activity-frame bg-surface">
              <ActivitySlot
                kind="requesting"
                :incoming="incomingThinking"
                :since="activitySince"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="incoming · shell"
            wide
          >
            <div class="gallery-frame gallery-activity-frame bg-surface">
              <ActivitySlot
                kind="requesting"
                :incoming="runningShellTool"
                :since="activitySince"
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
      <div class="h-[480px] overflow-hidden rounded-lg border border-line">
        <GalleryWorkPanel :work="editWork" />
      </div>
      <GallerySection
        title="Changed Files"
        note="A shell call lists the files it touched under its row: icon, name and line counts as pills that wrap. A new file carries a green dot. Pick a file to show that call’s edits in the panel; unavailable contents leave the diff blank. Past three rows the rest fold into +N files. The row folds the command and output on its own; the pills do not move."
      >
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack [--agent-pad-x:0px]">
            <GallerySpecimen
              v-for="item in fileChangeCases"
              :key="item.block.id"
              :variant="item.variant"
              wide
            >
              <ToolShellBlock
                v-model:open="changeCaseOpen[item.block.id]"
                :block="item.block"
                :input="parseToolInput(item.block.input)"
                :is-streaming="false"
              />
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'turns'">
      <GallerySection
        title="Turn"
        note="Requesting from the send, then each block rolls into the tail row. The message shows at once as it will stay, and its delivery is part of the same wait: when the server confirms it, the row keeps its word and its clock. Not Delivered fails the delivery instead: the message says so with Retry, which sends it again with the same ID and shows Requesting from then. Offline sends while the backend cannot be reached: the message says it waits to be sent, never that it failed, and goes with its ID once Demi is back. Resume, Retry and Connect wait for the server first."
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
        </div>
        <div class="gallery-frame h-[20rem] bg-surface">
          <SessionSurface ref="turnSurface">
            <div class="flex h-full min-h-0 flex-col">
              <AgentMessageList
                ref="turnList"
                class="min-h-0 flex-1"
                :conversation-id="turnFlow.state.id"
                :blocks="turnFlow.state.blocks"
                :pending-steers="[]"
                :queue="[]"
                :phase="turnFlow.state.phase"
                :load="turnFlow.state.load"
                :pending-action="turnFlow.state.pendingAction"
                :pending-submission="turnFlow.pendingSubmission.value"
                :bottom-offset="turnSurface?.dockHeight ?? 0"
                :persisted-scroll-state="undefined"
                read-only
                @retry-submission="turnFlow.retrySubmission"
              />
            </div>
            <template #dock>
              <SessionDock
                :show-scroll-to-bottom="!!turnList && !turnList.isAtBottom"
                @scroll-to-bottom="turnList?.scrollToBottom()"
              />
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
        note="The user compacted after the second answer. The divider shows there, where the compaction was triggered, though the summary goes in before that answer. While the pass runs the divider says so, before a steer that arrived meanwhile; a failed pass leaves its error record there. A Compact the user asked for ended no turn, so the dock offers no Resume for it (Session failures shows both cases)."
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
                @delete-pending-steer="productWould('Withdraw the Steer')"
                @interrupt-pending-steer="productWould('Send the Steer Now')"
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'windows'">
      <GallerySection
        title="Agents"
        note="Half-session inspect. Tabs are running children. The circle-stop icon and Stop All label stop every running agent. Completed opens the searchable finished list."
      >
        <div class="relative h-[24rem] min-h-0">
          <SubagentPanel
            v-model:active-id="exhibitAgentId"
            :agents="agents"
            @abort="sessionFlow.abortSubagents"
            @abort-agent="sessionFlow.abortSubagent"
            :dismiss-outside="false"
          />
        </div>
      </GallerySection>
      <GallerySection
        title="Terminals"
        note="The same window. Tabs are running jobs; the body is a read-only xterm with ANSI color from bun, rg and git. The watch tab’s output comes live: the terminal adds only what is new and keeps its scrollback, and after a burst longer than a frame holds, it shows the frame’s tail anew. Closing a running tab stops its command."
      >
        <div class="relative h-[24rem] min-h-0">
          <TerminalPanel
            v-model:active-id="exhibitTerminalId"
            :terminals="terminals"
            @abort="sessionFlow.abortTerminal"
            :dismiss-outside="false"
          />
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'states'">
      <GallerySection title="Transcript Entrance" note="Restored history appears without motion. Only blocks appended after restoration enter; switching or scrolling back to existing blocks does not replay their entrance.">
        <GalleryTranscriptEntrance />
      </GallerySection>
      <GallerySection title="Edit and Resend" note="Shared editor, durable confirmation and recovery states.">
        <GalleryMessageEditing />
      </GallerySection>
      <GallerySection
        title="Product Session"
        note="The shared product page, with local fixture state and handlers. The header gives its title up to 260px and cuts it short last: a longer title is cut to that and the buttons keep their names, and as the frame narrows, the directory button and then the host button become their icons, their names in tooltips, and they name themselves again once the title fits whole. Resize the frame from its corner. The Rename button beside the title opens a menu of two ways to name the conversation. Rename edits the title in place, as a double-click on the title does: Enter or clicking away keeps the new title, Escape or an empty one keeps the old. Detect Title asks the model for a title from the user’s messages. Nothing shows progress while it is written: the menu still opens and Rename still works, Detect Title is disabled, and when the title arrives it simply changes, unless the user renamed the conversation meanwhile, whose title wins. Detect Title then stays disabled, saying why, until the next message, which this specimen stands in for with a pause."
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
        note="First opening uses the loading pane until history arrives. History stays readable while models load or the connection opens. Switching back to a cached session is immediate and reuses its connection. A dropped connection in an open session uses the “Connecting” tail row. An unconfirmed send keeps its user message as it will stay; Retry sends it again with the same ID and says Requesting from then, through the confirmation, until the answer begins. New conversation opens a local draft immediately."
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
                :pending-submission="deliveryFlow.pendingSubmission.value"
                :bottom-offset="0"
                :persisted-scroll-state="undefined"
                read-only
                @retry-submission="deliveryFlow.retrySubmission"
              />
            </div>
            <Button size="sm" class="mt-2" @click="failDelivery">Fail Again</Button>
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
                  placeholder="Ask Demi…"
                  :providers="[]"
                  :models="{}"
                  model-load="loading"
                />
              </div>
            </div>
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
                  livePane === 'failed' ? 'The connection was reset.' : undefined
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
              <SessionStatus kind="none" />
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
        note="The app frame with the work panel open on the right: the sidebar, the session, and the panel are siblings, each side pane behind its own divider. The header’s panel control opens it and the panel’s fold control closes it, using the same 28px button, 14px icon, 12px right inset, tooltip and hover treatment as the conversation’s Open panel control; drag or double-click the divider on its left. The panel keeps a share of the width it splits with the session, so resizing the frame scales both while the sidebar keeps its px width; Reset Panel restores the initial selections."
      >
        <GallerySpecimen variant="frame · live" wide>
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
                  :account="demoAccount"
                  :projects="panelProjects"
                  :conversations="panelConversations"
                  :active-id="panelActiveConversationId"
                  :section-entries="sidebarEntries(PLUGIN_PAGES, () => true)"
                  @select="(id) => (panelActiveConversationId = id)"
                  @open-settings="(section) => productWould(section ? `Open ${section} Settings` : 'Open Settings')"
                />
              </template>
              <ChatSession
                :conversation="session"
                has-provider
                :aside-open="panelAsideOpen"
                :select-edit="(selection) => { panelWork.selectEdit(selection); panelAsideOpen = true }"
                :files="sessionFiles"
                @open-aside="panelAsideOpen = true"
                :pending-submission="sessionFlow.pendingSubmission.value"
                @retry-submission="sessionFlow.retrySubmission"
                @retry="sessionFlow.resume()"
                @rename="session.title = $event"
                @abort-subagents="sessionFlow.abortSubagents"
                @abort-subagent="sessionFlow.abortSubagent"
                @abort-terminal="sessionFlow.abortTerminal"
                @remove-queued="sessionFlow.removeQueued"
                @send-queued="sessionFlow.sendQueued"
                @remove-pending-steer="sessionFlow.removePendingSteer"
                @interrupt-pending-steer="sessionFlow.interruptPendingSteer"
              >
                <template #composer>
                  <GalleryComposer
                    placeholder="Ask Demi about the failing login test…"
                    :running="session.phase === 'running'"
                    :compacting="session.phase === 'compacting'"
            :usage="session.contextUsage ?? undefined"
                    @send="sessionFlow.turn"
                    @queue="sessionFlow.queue"
                    @stop="sessionFlow.stop"
                    @compact="sessionFlow.compact"
                  />
                </template>
              </ChatSession>
              <template #aside>
                <GalleryWorkPanel :work="panelWork" @close="panelAsideOpen = false" />
              </template>
            </SidebarLayout>
          </div>
        </GallerySpecimen>
        <div class="flex gap-2">
          <Button size="sm" @click="panelWork.reset">Reset Panel</Button>
          <!-- What the agent's `demi browser open` does, with `--show` or without, once its job ends. -->
          <Button size="sm" @click="agentOpens(true)">Open and Show a Page as the Agent</Button>
          <Button size="sm" @click="agentOpens(false)">Open a Page to Check as the Agent</Button>
          <span class="self-center font-mono text-[11px] text-fg-faint">panel {{ Math.round(panelAsideShare * 100) }}% of the width it splits with the session · at least {{ ASIDE_SHARE.minWidth }}px, leaving the session {{ ASIDE_SHARE.mainMinWidth }}px</span>
        </div>
      </GallerySection>
      <GallerySection
        title="Work Panel"
        note="Change and File are pinned tabs, the `change` and `file` kinds of the changes and file-browser plugins: content-sized buttons outside the tab strip that are never created, closed or saved, never scroll with the strip, and only compete with its tabs for the selection. In a panel too narrow for them and a tab, their titles truncate before the strip gives way, so the strip keeps room for a tab and a new tab always shows. The strip holds the user’s tabs, content-sized and capped at 160px, shrinking toward their mark and a few characters before it scrolls, with scrolling and close menus that affect only tabs. The add control opens a tab in the conversation’s browser: globe-plus while the strip is empty, a plain plus beside tabs. The new tab stands in the strip at once, selected, as what a new tab is: named New Tab, its address bar empty and a blank page, which its first picture replaces; the gallery’s browser takes about a second, as a host takes a moment. An address typed before then shows at once with a thin loading line, and the tab opens there. Reload, Back, Forward and an address show the loading line on the click, before anything left the page: the gallery’s browser answers about a second later and starts loading a moment after its answer, as a far host does. The host numbers its tab lists and the answer names the last one before the request, so only a list numbered higher ends the line, whichever reaches the page first; a page that loads keeps the picture it had under the line until the browser says it stopped loading. Back and Forward are unavailable while the tab’s history has no page that way; go to another address to make Back available. A request the browser refuses ends the line at once and is reported in a toast, never in the tab; a tab the plugin could not open shows a region status with Retry. What the Host’s view cannot do for the viewer while the picture still shows, such as delivering its input, is a toast once; a Host that cannot capture its tabs shows a region status in place of the picture, as the specimen of one does. A closed tab leaves at once and never comes back, fading out as an unselected tab while the selection passes on. While a tab loads, it shows so in three places at once, as a web browser does: Reload turns into Stop, the tab’s mark in the strip turns into a spinner, and the line runs; Stop asks the browser to stop the page, and the three end once the browser says it stopped and, for a tab that shows no picture yet, such as one opening or reopening, once its first picture arrives. Stop is never unavailable while a tab loads: pressed while the tab is still opening, it stops the page as soon as the tab exists. The gallery’s pages take five seconds to load, so Stop can be tried. A page the agent closed leaves the strip, as a closed tab does. A tab the browser lost when it ended, as a release or a stopped Cloud ends it, keeps its title and address, and opens its page again as soon as it is shown, loading as any page does; a reopening the host refuses shows its message with Retry, which returns the tab to loading at once. A view that reconnects keeps its last picture under a quiet note. On a host without the browser, which only the agent installs with demi browser install, the add control is disabled and its tip says so; the tabs already open stay. A viewer’s browser that cannot decode the host’s H.264, such as a Chromium built without proprietary codecs, opens no view: its browser tab says so in place of the picture, as the last specimen’s does in any browser. A page holds no view while it is hidden, behind another browser tab or in a minimized window, and opens a new one when it is shown again: switch away from the gallery and back, and the picture connects again, its moving mark starting over. A Web picture stands at its own size from the panel’s top-left corner and is never scaled: while the panel grows, the part it does not cover yet is white until a picture at the new size arrives, and while it shrinks, the panel cuts it; a phone’s picture is scaled to fit and centred. The cursor over the page is the page’s own, resolved here from where the page says each cursor applies, so moving across the button shows its pointer at once; no caret or local control shows over the picture. A tab selected again after another tab, or after File, shows its picture, address and title at once, with no loading line for a page that had loaded, and the File view’s tree shows as it was, with no slide. The agent shows a tab with demi browser show, or opens one with --show: once its job ends, the panel opens, closed or not, and selects that tab, once per showing, and the user’s own selection after it stands; a tab the agent opens only to check its own work joins the strip without taking the view (the app frame’s controls show both). The viewport control is a square icon button, a computer or a phone, as far from the address as the navigation group is; its menu rows carry the same icons. Change groups its added/removed counts with a 2px gap and shows uncommitted totals, none while nothing changed; picked again, it shows the view as it was left, its mode and file included; file pills still open retained edits there. A browser tab wears its page’s icon, as a web browser’s tab does, and the globe only for a page without one, such as the Docs tab’s site. Its menu offers Reload and Duplicate, which opens the address again in a tab right after it, before the Close commands. File uses a Lucide outline icon until a file is selected, then its file-type icon. Every tab content puts its address row immediately below the strip; the same divider as File and Change separates it from the page. Browser and File navigation buttons have no extra gap between them; both address bars leave 12px after the navigation group. A tab the user just made opens with its address focused and selected, waiting for where to go. A click into an address selects it whole, a second click places the caret, and Enter submits it and lets the field go, so keys reach the page again. The address bar reads what is typed as a browser’s does: localhost:3000 opens over http, example.com over https, words are a Google search, and Escape gives back the page’s address. With the focus in the tab, ⌘L focuses the address, ⌘[ and ⌘] go Back and Forward and ⌘R reloads the page, never Demi. The drawn page’s link opens the invoice in a new tab, which joins the strip at once and is selected, as a click’s new tab is. A right click on the page opens the browser’s menu: Back, Forward and Reload, a link’s Open Link in New Tab and Copy Link Address, Cut, Copy and Paste in the field; Escape closes it, as it closes every menu. A tab Open Link in New Tab opens, like one the page’s link opens, goes right after its page’s tab, behind the tabs that tab opened before, as Chrome places them, and keeps that order after a reload. Ship order asks in a dialog titled with the page’s host, which Enter and Escape answer. Download guide.pdf shows the download in a bubble under the address bar’s download button: its size as it comes, then Save, which saves it to this computer, and Show in Files, which opens it in File. Beside a new conversation, before its first message, the panel binds no plugin and says that files and changes appear after the first message; Close is its only control."
      >
        <div class="grid gap-6 md:grid-cols-2">
          <GallerySpecimen variant="tabs" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="exhibitWork" @close="exhibitWork.reset" />
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
              <Button size="sm" @click="failNextOpen">Refuse the Next Opening on the Device</Button>
              <span>Then end the browser: the reopening fails, and Retry loads again.</span>
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
          </GallerySpecimen>
          <GallerySpecimen variant="tabs · a viewer’s browser that cannot decode H.264 · live" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="undecodedWork" @close="productWould('The work panel closes.')" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="tabs · a Host without the browser · live" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <GalleryWorkPanel class="w-full" :work="noBrowserWork" @close="productWould('The work panel closes.')" />
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
        title="File View"
        note="A file of the workspace: the path as crumbs from the workspace root, the file itself, and the workspace tree beside it with the file selected. Text opens read-only in the code editor, colored by its language, with folding. Mod-f in the text opens a find bar below it: the query with its match count, “Match case”, “Match whole word” and “Use regular expression”, and “Previous match” and “Next match”, which Shift+Enter and Enter in the field also do. Typing selects the first match from the selection on, the count shows a question mark while the selection is on no match, a count past 9999 stops there with a +, and the scrollbar marks every match counted until Escape or Close shuts the bar. An image fits the pane without being enlarged, over a checkerboard where it is transparent, and a click shows it at its actual size; video and audio play in the browser’s own player and PDF in its own viewer, each with its pixel size and file size under it. Markdown renders like a repository file on GitHub: its HTML sanitized, so the script and the handler at the end of the README never run, its math and code rendered, its front matter a YAML block, and its links opening files here, scrolling to headings, or leaving for the web. Markdown and SVG switch between Preview and Source. A file that is neither text nor previewable is a card with its facts and Download, and every file has Download in the header. A crumb opens a menu of what lies beside it, directories unfolding into their own; a file picked there, clicked in the tree or linked from a document replaces the one shown, and Back and Forward before the crumbs walk the files shown. A click on the crumb row anywhere but a crumb turns it into a text field with the path as the crumbs read it, from the workspace, never the Host’s own directories above it; a relative path starts from the workspace, and the field completes from its folders and files as it is typed, a completed file opening at once: Enter opens a file, or goes to a folder as Finder’s Go to Folder does, unfolding it in the tree with the folders above it and selecting it until another file opens; a folder outside the workspace says the tree shows the workspace only. A click on a row gives the tree the keyboard: the arrows, Home, End and Enter move through it and act, and typing a name moves to the first row shown that starts with it, opening nothing. A right-click in the tree downloads a file, or uploads into a folder, or into the workspace from the empty space; the uploads list under the tree, moving here at a pace slow enough to watch. The control at the end of the crumb row hides and shows the tree; it grows from the end as the file gives way, and shrinks back, and a tree hidden and shown again is as it was left. A view that would keep less than 320px beside the tree hides it by itself; the control then slides the tree in over the file, which stays as it is, and the control, a click beside the tree or a file picked in it slides it away. The tree docks again once the view is wide enough, and its divider stops where the file would get narrower than that; the narrow frame resizes from its corner. Reads carry the fixture’s latency, so the text and each directory show their loading state the first time; the fixture’s Host watches its files, as the product’s does, so a file or folder shown again shows at once, scrolled where it was, and asks nothing. Edit cookie.ts Outside Demi writes the file as another program would: the Host reports it, and the text is read again and replaced in place, without a loading state, the scroll and folds staying. A read again that fails keeps the text and says so quietly over it, in the page’s own words, with Retry, and the view reads it again by itself once the Host is back and its watch live. An image a document names that does not load shows its alt text in a dashed box, as the README’s missing one does, and tries again when the document is read again. server.log is over the text route’s 8 MiB: it opens as its first 8 MiB, with a line saying how much of how much shows and Download. A Host that cannot watch its files has the view say that it shows them as last read, with Refresh, and every file shown is checked."
      >
        <div class="flex flex-wrap gap-1">
          <Button v-for="file in previewFiles" :key="file" size="sm" @click="showInFileView(`${workspace.root}/${file}`)">{{ file }}</Button>
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
        title="Change View"
        note="Diffs from one of two sources, the switch in the header picks. A changed image, video, audio file or PDF shows its committed version beside the working tree’s instead, each with its sizes, a new file only the second; a binary file with no preview shows a card per side with Download, and a committed version over 8 MiB says it is too large, with no Download. Markdown and SVG switch between the text diff and Preview, which renders both sides, labeled “Committed” and “Working tree”, or “Before” and “After” in Conversation. Uncommitted is the working tree against the last commit: the diff of the selected file beside the tree of the files git status lists, each ending its row with its line counts and the letter VS Code’s Git marks it with, by VS Code’s own rules from git’s two status letters: “U” untracked, “A” added, “M” modified, “D” deleted (its name struck through), “R” renamed, “T” type changed, and an exclamation mark in conflict, in VS Code’s colors and with VS Code’s words as the tooltip; where git has a letter for both the index and the working tree, the working tree’s shows, so a staged new file edited again is M. The fixtures hold every mark. The files and lines are summed up in the tree’s caption. Conversation shows only the file picked under a shell call, without a file tree or a list source. It shows that file’s retained edits, with a segment control when other calls wrote between them. Missing contents leave the diff blank. Back and Forward walk what the view has shown, across modes. The header also opens the selected file itself. Only Uncommitted offers a tree toggle, and in a narrow view its tree hides and shows over the diff the way the File view’s does; its tree’s caption lists the changes again, its control turning while the list is on its way. A header over the diff names the file shown, in either mode, as GitHub’s diff header does: its icon, its folder from the workspace, its name, where a renamed file came from, and its line counts. Picking a file pill opens Conversation; the Change tab picked again shows the view as it was left; with nothing picked, Conversation says how to fill it. Under Uncommitted, a workspace outside a Git repository shows no file tree or toggle, only “Not a git repository.” It keeps the last list when a listing failed, and says under the rows when the list was cut short. A host can name the workspace in place of its directory’s name, as the product does for the Cloud’s own session directory. The tree takes the keyboard as the File View’s does: typing a name moves to the first row shown that starts with it, and Enter selects a file."
      >
        <GallerySpecimen
          v-for="specimen in [
            { variant: 'uncommitted · live', work: changeUncommitted, rootName: undefined, narrow: false },
            { variant: 'uncommitted · narrow, the tree hides, its control shows it over the diff', work: changeUncommitted, rootName: undefined, narrow: true },
            { variant: 'uncommitted · not a repository, named Workspace', work: changeNoRepository, rootName: 'Workspace', narrow: false },
            { variant: 'uncommitted · listing failed, cut short', work: changeStale, rootName: undefined, narrow: false },
            { variant: 'conversation · picked', work: changePicked, rootName: undefined, narrow: false },
            { variant: 'conversation · a Markdown file picked', work: changeDocument, rootName: undefined, narrow: false },
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
              :changes="specimen.work.changes.value"
              :root="workspace.root"
              :root-name="specimen.rootName"
              :can-back="specimen.work.tab.value.back.length > 0"
              :can-forward="specimen.work.tab.value.forward.length > 0"
              @update:mode="specimen.work.setMode"
              @update:selected="specimen.work.select"
              @back="specimen.work.back"
              @forward="specimen.work.forward"
            />
          </div>
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'session'">
      <ChatSession
        :conversation="session"
        has-provider
        :select-edit="(selection) => { panelWork.selectEdit(selection); panelAsideOpen = true; view = 'panel' }"
        :files="sessionFiles"
        :edit-version="editVersion"
        :permission-requests="sessionRequests"
        @decide-permission="(id, decision) => (sessionRequests = decidePermission(sessionRequests, id, decision))"
        v-model:message-edit="messageEdit"
        :pending-submission="sessionFlow.pendingSubmission.value"
        @retry-submission="sessionFlow.retrySubmission"
        @retry="sessionFlow.resume()"
        @rename="session.title = $event"
        @save-scroll="(_id, state) => (session.scroll = state)"
        @abort-subagents="sessionFlow.abortSubagents"
        @abort-subagent="sessionFlow.abortSubagent"
        @abort-terminal="sessionFlow.abortTerminal"
        @remove-queued="sessionFlow.removeQueued"
        @send-queued="sessionFlow.sendQueued"
        @remove-pending-steer="sessionFlow.removePendingSteer"
        @interrupt-pending-steer="sessionFlow.interruptPendingSteer"
      >
        <template #composer>
          <GalleryComposer
            v-model:message-edit="messageEdit"
            @submit-edit="submitEdit"
            placeholder="Ask Demi about the failing login test…"
            :running="session.phase === 'running'"
            :compacting="session.phase === 'compacting'"
            :usage="session.contextUsage ?? undefined"
            :archived="session.archived"
            @restore="session.archived = false"
            @send="sessionFlow.turn"
            @queue="sessionFlow.queue"
            @stop="sessionFlow.stop"
            @compact="sessionFlow.compact"
          />
        </template>
      </ChatSession>
    </template>
  </div>
</template>

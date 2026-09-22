<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { Play } from '@lucide/vue'
import ThinkingBlock from '@demicodes/web-ui/agent/blocks/ThinkingBlock.vue'
import AgentReceiptBlock from '@demicodes/web-ui/agent/blocks/AgentReceiptBlock.vue'
import { agentReceiptMessages } from '../fixtures/blocks'
import ErrorBlock from '@demicodes/web-ui/agent/blocks/ErrorBlock.vue'
import ToolShellBlock from '@demicodes/web-ui/agent/blocks/ToolShellBlock.vue'
import { parseToolInput } from '@demicodes/web-ui/agent/block-helpers'
import ActivitySlot from '@demicodes/web-ui/agent/blocks/ActivitySlot.vue'
import type { ActivityKind, HandoffBlock } from '@demicodes/web-ui/agent/activity-slot'
import { provideEditSelection } from '@demicodes/web-ui/agent/edit-selection'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import { changeWorkTab, changeTabPath, workPanelTabs, findChangeWorkTab, goBackInTab, goForwardInTab, showChangeInTab, showCallEdit, showFileInTab, type ChangeWorkTab, type WorkTab } from '@demicodes/web-ui/agent/work-panel'
import { addTab, emptyPanelState, removeTabs, selectInPanel, selectedTab, updateTab, type PanelState } from '@demicodes/web-ui/agent/panel-tabs'
import type { PanelTabKind } from '@demicodes/web-ui/agent/panel-kinds/kind'
import { pageTabKind } from '@demicodes/web-ui/agent/panel-kinds/page'
import { exposePageTab } from '@demicodes/web-ui/agent/panel-kinds/page-data'
import { browserTabKind } from '@demicodes/web-ui/browser/kind'
import { BrowserTabsController, browserTabDataSchema } from '@demicodes/web-ui/browser/tabs'
import { callChangeSource, type CallEditSelection, type ChangeMode, type ChangeSources } from '@demicodes/web-ui/files/changes'
import ChangeView from '@demicodes/web-ui/files/ChangeView.vue'
import FileView from '@demicodes/web-ui/files/FileView.vue'
import { createGalleryChangeSet, createGalleryWorkspace } from '../fixtures/workspace'
import { galleryBrowserTabs } from '../fixtures/live-browser'
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
import PendingSubmission from '@demicodes/web-ui/agent/PendingSubmission.vue'
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
import { firstRunningTerminalId } from '@demicodes/web-ui/agent/terminals'
import type { ThinkingConfig, UserContentBlock } from '@demicodes/core'
import { composerAttachment, encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Button from '@demicodes/web-ui/ui/Button.vue'
import {
  demoImageUrl,
  demoModel,
  changesDemoBlocks,
  editingShellTool,
  fileChangeCases,
  runningShellTool,
  shellTool,
  thinkingText,
  steerPrompt,
  transcriptDemoBlocks,
} from '../fixtures/blocks'
import {
  demoModels,
  demoProviders,
  mediumThinking,
  offlineProviders,
  openaiOnlyProviders,
  usageAt,
} from '../fixtures/catalog'
import { gallerySubagents } from '../fixtures/subagents'
import { galleryTerminals } from '../fixtures/terminals'
import { useTurnFlow, type TurnFlowKind } from '../turn-flow'
import GalleryComposer from '../components/GalleryComposer.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import GalleryTabStripDrive from '../components/GalleryTabStripDrive.vue'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()
const historyModelBlocks = transcriptDemoBlocks()
const submissionError = ref<string | null>('Connection closed before confirmation')

const messageEdit = ref<MessageEditState | null>(null)
const editRevision = ref(0)
const compacting = ref(false)
const agents = reactive(gallerySubagents())
const terminals = reactive(galleryTerminals())
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
const panelAsideOpen = ref(true)
const panelProjects = ref(demoProjects())
const panelConversations = ref(demoConversations())
const panelActiveConversationId = ref<string | null>('c-login')
/**
 * One work-panel specimen's state, held by the gallery as the product's store
 * holds it: the fixed views, the selection and tabs, and the `browser` kind
 * over the gallery's own browser.
 */
function useWorkTabs(selection: string, path = 'src/auth/cookie.ts') {
  const views = ref(workPanelTabs(path))
  const panel = ref<PanelState>({ ...emptyPanelState(), selection })
  const browser = new BrowserTabsController(galleryBrowserTabs(), {
    bound: () => panel.value.tabs.flatMap((tab) => {
      const parsed = tab.kind === 'browser' ? browserTabDataSchema.safeParse(tab.data) : null
      return parsed?.success ? [parsed.data] : []
    }),
    add: (data) => {
      panel.value = addTab(panel.value, { kind: 'browser', data }, { select: false }).state
    },
  })
  onBeforeUnmount(() => browser.dispose())
  const kinds: PanelTabKind[] = [browserTabKind(browser), pageTabKind]
  function select(next: string) {
    panel.value = selectInPanel(panel.value, next)
  }
  function add(kind: string, data: unknown) {
    panel.value = addTab(panel.value, { kind, data }, { select: true }).state
  }
  function update(id: string, data: unknown) {
    panel.value = updateTab(panel.value, id, data)
  }
  /** A closed tab goes at once; its kind then does what a closed tab of it needs. */
  function closeTabs(ids: string[]) {
    const closing = panel.value.tabs.filter((tab) => ids.includes(tab.id))
    panel.value = removeTabs(panel.value, ids)
    for (const tab of closing) {
      const kind = kinds.find((candidate) => candidate.kind === tab.kind)
      const parsed = kind?.schema.safeParse(tab.data)
      if (kind?.removed && parsed?.success) {
        kind.removed(parsed.data)
      }
    }
  }
  function showChange(id: string, mode: ChangeMode, path: string | null, selection?: { call: ChangeWorkTab['call']; edit: number }) {
    views.value = showChangeInTab(views.value, id, mode, path, selection)
  }
  /** A file by workspace path, shown in the File view in place. */
  function open(path: string) {
    const next = showFileInTab(views.value, path)
    views.value = next.tabs
    if (next.activeId !== null) {
      select(next.activeId)
    }
  }
  function back(id: string) {
    views.value = goBackInTab(views.value, id)
  }
  function forward(id: string) {
    views.value = goForwardInTab(views.value, id)
  }
  function selectEdit(selection: CallEditSelection) {
    const next = showCallEdit(views.value, selection)
    views.value = next.tabs
    select(next.activeId)
  }
  function reset() {
    views.value = workPanelTabs(path)
    panel.value = { ...emptyPanelState(), selection }
  }
  return { views, panel, kinds, browser, select, add, update, closeTabs, open, showChange, selectEdit, back, forward, reset }
}
const workspace = createGalleryWorkspace()
const fileViewTree = ref(true)
const changeViewTree = ref(true)
const fileViewMode = ref<'preview' | 'source'>('preview')
const changeViewPresentation = ref<'diff' | 'preview'>('diff')
/** One of each kind the File view previews, by workspace path. */
const previewFiles = [
  'README.md', 'assets/logo.svg', 'assets/photo.png', 'assets/demo.mp4',
  'assets/tone.m4a', 'docs/guide.pdf', 'dist/app.zip', 'src/auth/cookie.ts',
]
/** One change tab on its own, stepped the way the panel steps the host's: for the Change view specimens. */
function useChangeTab(mode: ChangeMode, path: string | null, changes: ChangeSources) {
  const tabs = ref<WorkTab[]>(showChangeInTab([changeWorkTab('c', mode)], 'c', mode, path, { call: changes.conversation, edit: 0 }))
  const tab = computed(() => findChangeWorkTab(tabs.value)!)
  /** The file the tab holds in its mode, else the first there is. */
  const selected = computed(() => changeTabPath(tab.value, tab.value.mode, changes.uncommitted.files))
  function show(mode: ChangeMode, path: string | null) {
    tabs.value = showChangeInTab(tabs.value, 'c', mode, path)
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
    setMode: (mode: ChangeMode) => show(mode, changeTabPath(tab.value, mode)),
    select: (path: string | null) => show(tab.value.mode, path),
    back: () => (tabs.value = goBackInTab(tabs.value, 'c')),
    forward: () => (tabs.value = goForwardInTab(tabs.value, 'c')),
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
const panelWork = useWorkTabs('change')
/** The session's messages reach the gallery workspace: images from its fixtures, files opened in the frame's panel. */
const sessionFiles: ConversationFiles = {
  imageUrl: (path) => workspace.source.contents.url(path),
  open: (path) => {
    panelWork.open(path)
    panelAsideOpen.value = true
    view.value = 'panel'
  },
}
const exhibitWork = useWorkTabs('file')
const editWork = useWorkTabs('change')
// The gallery's own browser stands behind every specimen's `browser` kind; this one lists its tabs.
void editWork.browser.refresh()
provideEditSelection(editWork.selectEdit)
async function readCallChange(commandId: string, path: string, edit: number) {
  return {
    original: `// ${path}\nconst cookie = '${edit === 0 ? 'sid' : 'session'}'\n`,
    modified: `// ${path}\nconst cookie = '${edit === 0 ? 'session' : 'session-v2'}' // ${commandId}\n`,
  }
}
const changeUncommitted = useChangeTab('uncommitted', 'src/auth/cookie.ts', { uncommitted: workspace.changes, conversation: null })
const changePicked = useChangeTab('conversation', 'src/auth/cookie.ts', {
  uncommitted: workspace.changes,
  conversation: callChangeSource({
    commandId: 'gallery-cookie-edit',
    file: { path: 'src/auth/cookie.ts', kind: 'modified', added: 1, removed: 1, edits: [{ kept: true }] },
  }, readCallChange),
})
const changeDocument = useChangeTab('conversation', 'README.md', {
  uncommitted: workspace.changes,
  conversation: callChangeSource({
    commandId: 'gallery-readme-edit',
    file: { path: 'README.md', kind: 'modified', added: 0, removed: 1, edits: [{ kept: true }] },
  }, (_commandId, path, _edit, signal) => workspace.changes.read(path, signal)),
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
// A page opened the way an expose row opens it, with the expose glyph, beside the browser's own tabs.
const browserWork = useWorkTabs('change', '')
void browserWork.browser.refresh()
browserWork.add(pageTabKind.kind, exposePageTab({
  url: `data:text/html,${encodeURIComponent('<body style="font:14px system-ui;padding:24px"><h1>Dev server</h1><p>A page shown in the tab\'s sandboxed frame.</p><a href="https://example.com" target="_blank">A link that opens a popup</a></body>')}`,
  address: '127.0.0.1:5173',
}))
/** The browser tab the specimen shows, as its kind reads it. */
const shownBrowserTab = computed(() => {
  const tab = selectedTab(browserWork.panel.value)
  const parsed = tab?.kind === 'browser' ? browserTabDataSchema.safeParse(tab.data) : null
  return parsed?.success ? (parsed.data.tab ?? null) : null
})
/** Closes the page behind the panel's back, as the agent's `close` would, and reads the list the way a finished tool call does. */
async function closeOnDevice() {
  const tab = shownBrowserTab.value
  if (tab === null) {
    return
  }
  await browserWork.browser.api.close(tab)
  await browserWork.browser.refresh()
}
let nextQueue = 3
let nextSent = 1

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
  {
    id: 'q1',
    text: 'Also add a case for the expired cookie.',
    content: [{ type: 'text', text: 'Also add a case for the expired cookie.' }],
  },
  {
    id: 'q2',
    text: 'Keep the light-mode screenshot in the same PR.',
    content: [{ type: 'text', text: 'Keep the light-mode screenshot in the same PR.' }],
  },
]
const streamFlow = useTurnFlow({ id: 'gallery-stream' })
const turnFlow = useTurnFlow({ id: 'gallery-turn' })
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

const selectorProvider = ref('anthropic')
const selectorModel = ref('claude-sonnet')
const selectorThinking = ref<ThinkingConfig>(mediumThinking)
const selectorTier = ref<string | null>(null)
const fastProvider = ref('anthropic')
const fastModel = ref('claude-sonnet')
const fastThinking = ref<ThinkingConfig>(mediumThinking)
const fastTier = ref<string | null>('priority')

function onSelectSelectorModel(providerId: string, modelId: string): void {
  selectorProvider.value = providerId
  selectorModel.value = modelId
}

function onSelectFastModel(providerId: string, modelId: string): void {
  fastProvider.value = providerId
  fastModel.value = modelId
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
      data: new Uint8Array(),
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
const functionalShellFiles = ref(false)
const changeCaseOpen = reactive<Record<string, boolean>>({})
const functionalThinkingStartedAt = new Date().toISOString()
const functionalThinkingEndedAt = new Date(
  Date.parse(functionalThinkingStartedAt) + 8_000,
).toISOString()
// Requesting covers a recovery in flight and the agent's own retries: one wait, one word.
const activityKinds: ActivityKind[] = ['requesting', 'connecting']
const incomingThinking: HandoffBlock = {
  type: 'thinking',
  id: 'incoming-thinking',
  createdAt: functionalThinkingStartedAt,
  model: demoModel,
  text: '',
  signature: null,
}

function takePendingSteer(id: string) {
  const pending = session.pendingSteers.find((candidate) => candidate.id === id)
  session.pendingSteers = session.pendingSteers.filter((candidate) => candidate !== pending)
  return pending
}

function deletePendingSteer(id: string): void {
  takePendingSteer(id)
}

function interruptPendingSteer(id: string): void {
  const pending = takePendingSteer(id)
  if (!pending) {
    return
  }
  sessionFlow.stop()
  session.blocks = [
    ...session.blocks,
    {
      type: 'steer',
      id: `steer-sent-${nextSent++}`,
      turnId: 'turn-gallery',
      createdAt: new Date().toISOString(),
      model: demoModel,
      content: pending.content,
    },
  ]
  sessionFlow.turn(pending.content)
}

function queueDraft(content: UserContentBlock[]): void {
  session.queue.push({
    id: `q${nextQueue++}`,
    text: content.flatMap((part) => part.type === 'text' ? [part.text] : []).join(''),
    content,
  })
}

function removeQueued(id: string): void {
  session.queue = session.queue.filter((entry) => entry.id !== id)
}

function sendNow(id: string): void {
  const item = session.queue.find((entry) => entry.id === id)
  if (!item) {
    return
  }
  session.queue = session.queue.filter((entry) => entry.id !== id)
  session.pendingSteers = [
    ...session.pendingSteers,
    { id: `gallery-${nextSent++}`, content: item.content },
  ]
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

function compact(): void {
  compacting.value = true
  window.setTimeout(() => {
    compacting.value = false
  }, 1200)
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
function abortAgent(id: string) {
  const agent = agents.find((entry) => entry.id === id)
  if (agent && agent.phase === 'running') {
    agent.phase = 'aborted'
    agent.endedAt = new Date().toISOString()
  }
}
function abortAgents() {
  for (const agent of agents) {
    abortAgent(agent.id)
  }
}
function abortTerminal(id: string) {
  const terminal = terminals.find((entry) => entry.id === id)
  if (terminal && terminal.phase === 'running') {
    terminal.phase = 'exited'
    terminal.endedAt = new Date().toISOString()
  }
}
</script>

<template>
  <div :class="fillPane ? 'flex min-h-0 flex-1 flex-col' : 'flex flex-col gap-10'">
    <template v-if="view === 'tabs'">
      <GallerySection
        title="Motion"
        note="The work panel's browser tabs, and every motion their strip owns, in a narrow frame that overflows and one at the pane width. Tabs fit their mark and title up to 160px; longer titles truncate. A tab opens by growing from nothing and closes by collapsing, while the New browser tab control follows the last tab until they scroll and then holds the right edge. Selecting or opening a tab that is cut off scrolls, with the same timing as the tab, until it shows whole and clear of the fade; closing scrolls back when the end comes into reach. Play all runs through the cases; the tabs, their close controls and their menus work too."
      >
        <GallerySpecimen variant="narrow · overflows" wide>
          <GalleryTabStripDrive width="32rem" />
        </GallerySpecimen>
        <GallerySpecimen variant="wide · pane width" wide>
          <GalleryTabStripDrive />
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'composer'">
      <GallerySection
        title="Composer"
        note="Idle through Fast Mode; text formatted as it is typed; each file a capsule where it was put, through its upload phases, a failed upload with Retry, and a remote host file naming its device; and queue. Enter on a message that cannot go shows the send button's reason where the pointer would. A message opens the composer as soon as it needs more than the line it is on, whether it holds lines of its own or its text outgrows the width, and closes it again when it fits; nothing is ever cut off at the line's end. One send; a running turn queues. An unavailable last model keeps the chip, warns, and blocks send. No usable model and an archived conversation both replace the input with the same snackbar: a line on the left, Configure models or Restore conversation on the right. A conversation its Cloud holds during a reset uses that snackbar with a spinner and no action, and the input returns with its draft when the reset ends. The input keeps at least 128px: where the model chip's name and level would leave it less, the chip is its sparkle alone, the name and level in its tooltip, and it names the model again once there is room; loading and a failed load give way the same. The narrow composer resizes from its corner."
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
            <div class="max-w-full resize-x overflow-hidden pb-3" style="width: 343px; min-width: 14rem">
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
            variant="queue"
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
        title="ModelSelector"
        note="The model dropdown aligns to the trigger’s right edge, extending to the left. A new conversation inherits the saved model, reasoning and Fast Mode; the product adapter persists this preference to the backend."
      >
        <div class="specimen-stack">
          <GallerySpecimen variant="new conversation · saved choice">
            <GalleryModelPreference />
          </GallerySpecimen>
          <GallerySpecimen variant="chip">
            <ModelSelector
              :providers="demoProviders"
              :models="demoModels"
              :selected-provider-id="selectorProvider"
              :selected-model-id="selectorModel"
              :thinking-config="selectorThinking"
              :service-tier-id="selectorTier"
              @select-model="onSelectSelectorModel"
              @change-thinking="selectorThinking = $event"
              @change-service-tier="selectorTier = $event"
            />
          </GallerySpecimen>
          <GallerySpecimen variant="fast">
            <ModelSelector
              :providers="demoProviders"
              :models="demoModels"
              :selected-provider-id="fastProvider"
              :selected-model-id="fastModel"
              :thinking-config="fastThinking"
              :service-tier-id="fastTier"
              @select-model="onSelectFastModel"
              @change-thinking="fastThinking = $event"
              @change-service-tier="fastTier = $event"
            />
          </GallerySpecimen>
          <GalleryOverlayWell size="wide">
            <GallerySpecimen variant="menu">
              <ModelMenu
                :providers="demoProviders"
                :models="demoModels"
                :selected-provider-id="selectorProvider"
                :selected-model-id="selectorModel"
                :thinking-config="selectorThinking"
                :service-tier-id="selectorTier"
                @select-model="onSelectSelectorModel"
                @change-thinking="selectorThinking = $event"
                @change-service-tier="selectorTier = $event"
              />
            </GallerySpecimen>
          </GalleryOverlayWell>
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
        title="Assistant message"
        note="Intermediate updates have no copy, fork or timestamp toolbar. Final replies keep their toolbar, including while a later user request runs. Preview fork success, pending creation, and a retry after failure."
      >
        <GallerySpecimen variant="message footer" wide>
          <GalleryAssistantMessages />
        </GallerySpecimen>
      </GallerySection>
      <GallerySection
        title="UserBlock"
        note="User; files as capsules where they were put, a picture or a text file's opening lines shown when pointed at; formatting; pending steer, queued, and stuck. Long messages have a view of their own: Lengths."
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
                pending
                deletable
                interruptible
                actions-pinned
                @delete="pendingSteerShown = false"
                @interrupt="pendingSteerShown = false"
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
                pending
                deletable
                sendable
                actions-pinned
                @delete="queuedShown = false"
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

      <GallerySection title="AgentReceiptBlock" note="Agent updates and completion receipts. Expand to read the message; these rows have no human message controls.">
        <div class="gallery-frame gallery-block-frame bg-surface">
          <div class="specimen-stack [--agent-pad-x:0px]">
            <GallerySpecimen
              v-for="(message, index) in agentReceiptMessages"
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
        note="Thinking, shell (collapsed and expanded), loading, and error."
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
        title="ActivitySlot"
        note="Requesting while the provider is asked, a Resume or Continue included, so a slow model reads as the provider's wait. The word and its clock stay through the agent's own retries after a failed attempt: they are the same wait, and a row that changed its word with each attempt would flicker. Elapsed time shows after one second; incoming blocks roll into the same row."
      >
        <div class="specimen-stack">
          <GallerySpecimen
            v-for="kind in activityKinds"
            :key="kind"
            :variant="kind"
            wide
          >
            <div class="gallery-frame gallery-activity-frame bg-surface">
              <ActivitySlot :kind="kind" />
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
        title="A heavy turn"
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
        <WorkPanel
          :views="editWork.views.value" :panel="editWork.panel.value" :kinds="editWork.kinds"
          :workspace="workspace" :read-call-change="readCallChange"
          @select="editWork.select" @add-tab="editWork.add" @update-tab="editWork.update" @close-tabs="editWork.closeTabs" @show-change="editWork.showChange" @open="editWork.open"
          @back="editWork.back" @forward="editWork.forward"
        />
      </div>
      <GallerySection
        title="Changed files"
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
        note="Requesting, then each block rolls into the tail row; Resume, Retry and Connect wait for the server first."
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
                :bottom-offset="turnSurface?.dockHeight ?? 0"
                :persisted-scroll-state="undefined"
                read-only
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
    </template>

    <template v-if="view === 'windows'">
      <GallerySection
        title="Agents"
        note="Half-session inspect. Tabs are running children. The circle-stop icon and Stop all label stop every running agent. Completed opens the searchable finished list."
      >
        <div class="relative h-[24rem] min-h-0">
          <SubagentPanel
            v-model:active-id="exhibitAgentId"
            :agents="agents"
            @abort="abortAgents"
            @abort-agent="abortAgent"
            :dismiss-outside="false"
          />
        </div>
      </GallerySection>
      <GallerySection
        title="Terminals"
        note="The same window. Tabs are running jobs; the body is a read-only xterm with ANSI color from bun, rg and git."
      >
        <div class="relative h-[24rem] min-h-0">
          <TerminalPanel
            v-model:active-id="exhibitTerminalId"
            :terminals="terminals"
            @abort="abortTerminal"
            :dismiss-outside="false"
          />
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'states'">
      <GallerySection title="Transcript entrance" note="Restored history appears without motion. Only blocks appended after restoration enter; switching or scrolling back to existing blocks does not replay their entrance.">
        <GalleryTranscriptEntrance />
      </GallerySection>
      <GallerySection title="Edit and resend" note="Shared editor, durable confirmation and recovery states.">
        <GalleryMessageEditing />
      </GallerySection>
      <GallerySection
        title="Product session"
        note="The shared product page, with local fixture state and handlers. The header gives its title up to 260px and cuts it short last: a longer title is cut to that and the buttons keep their names, and as the frame narrows, the directory button and then the host button become their icons, their names in tooltips, and they name themselves again once the title fits whole. Resize the frame from its corner. The Rename button beside the title opens a menu of two ways to name the conversation. Rename edits the title in place, as a double-click on the title does: Enter or clicking away keeps the new title, Escape or an empty one keeps the old. Detect title asks the model for a title from the user's messages: the Rename button is busy while it is written, the title changes, and Detect title is disabled, saying why, until the next message, which this specimen stands in for with a pause."
      >
        <div class="max-w-full resize-x overflow-hidden pb-3" style="min-width: 20rem">
          <GalleryConnectedSession />
        </div>
      </GallerySection>
      <GallerySection
        title="Scroll control without task chips"
        note="Scroll up, then return to the bottom. The left-aligned arrow fades inside a permanent control row. Transcript padding includes that row even when the arrow is hidden."
      >
        <GalleryConnectedSession :show-activity="false" />
      </GallerySection>
      <GallerySection
        title="Session load"
        note="First opening uses the loading pane until history arrives. History stays readable while models load or the connection opens. Switching back to a cached session is immediate and reuses its connection. A dropped connection in an open session uses the Connecting tail row. An unconfirmed send keeps its user message and retries with the same ID. New conversation opens a local draft immediately."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen variant="unconfirmed send · retry the same message" wide>
            <div class="gallery-frame bg-surface">
              <PendingSubmission
                id="pending-example"
                :text="`Review the notes ${ATTACHMENT_MARK} before the next run.`"
                :attachments="[composerAttachment({ name: 'notes.txt', phase: 'ready' })]"
                :sending="submissionError === null"
                :error="submissionError"
                @retry="submissionError = null"
              />
            </div>
            <Button size="sm" class="mt-2" @click="submissionError = 'Connection closed before confirmation'">Simulate failure</Button>
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
              <GalleryComposer
                placeholder="Ask Demi…"
                :providers="[]"
                :models="{}"
                model-load="loading"
              />
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
              <ActivitySlot kind="connecting" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="reconnecting · cached"
            wide
          >
            <div
              class="gallery-frame flex h-[16rem] flex-col overflow-hidden bg-surface"
            >
              <div class="min-h-0 flex-1 overflow-y-auto pt-2">
                <UserBlock :content="userBubble" />
                <ActivitySlot kind="connecting" />
              </div>
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
              <div
                v-else
                class="min-h-0 flex-1 overflow-y-auto pt-2"
              >
                <UserBlock :content="userBubble" />
              </div>
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
        title="The frame"
        note="The app frame with the work panel open on the right: the sidebar, the session, and the panel are siblings, each side pane behind its own divider. The header's panel control opens it and the panel's fold control closes it, using the same 28px button, 14px icon, 12px right inset, tooltip and hover treatment as the conversation’s Open panel control; drag or double-click the divider on its left. The panel keeps a share of the width it splits with the session, so resizing the frame scales both while the sidebar keeps its px width; Reset panel restores the initial selections."
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
                  :account="demoAccount"
                  :projects="panelProjects"
                  :conversations="panelConversations"
                  :active-id="panelActiveConversationId"
                  @select="(id) => (panelActiveConversationId = id)"
                />
              </template>
              <ChatSession
                :conversation="session"
                has-provider
                :aside-open="panelAsideOpen"
                :select-edit="(selection) => { panelWork.selectEdit(selection); panelAsideOpen = true }"
                :files="sessionFiles"
                @open-aside="panelAsideOpen = true"
                @retry="sessionFlow.resume()"
                @rename="session.title = $event"
                @abort-subagents="abortAgents"
                @abort-subagent="abortAgent"
                @abort-terminal="abortTerminal"
                @remove-queued="removeQueued"
                @send-queued="sendNow"
                @remove-pending-steer="deletePendingSteer"
                @interrupt-pending-steer="interruptPendingSteer"
              >
                <template #composer>
                  <GalleryComposer
                    placeholder="Ask Demi about the failing login test…"
                    :running="session.phase === 'running'"
                    @send="sessionFlow.turn"
                    @queue="queueDraft"
                    @stop="sessionFlow.stop"
                  />
                </template>
              </ChatSession>
              <template #aside>
                <WorkPanel
                  :read-call-change="readCallChange"
                  :views="panelWork.views.value" :panel="panelWork.panel.value" :kinds="panelWork.kinds"
                  :workspace="workspace"
                  @select="panelWork.select"
                  @add-tab="panelWork.add" @update-tab="panelWork.update" @close-tabs="panelWork.closeTabs"
                  @show-change="panelWork.showChange"
                  @open="panelWork.open"
                  @back="panelWork.back"
                  @forward="panelWork.forward"
                  @close="panelAsideOpen = false"
                />
              </template>
            </SidebarLayout>
          </div>
        </GallerySpecimen>
        <div class="flex gap-2">
          <Button size="sm" @click="panelWork.reset">Reset panel</Button>
          <span class="self-center font-mono text-[11px] text-fg-faint">panel {{ Math.round(panelAsideShare * 100) }}% of the width it splits with the session · at least {{ ASIDE_SHARE.minWidth }}px, leaving the session {{ ASIDE_SHARE.mainMinWidth }}px</span>
        </div>
      </GallerySection>
      <GallerySection
        title="Work panel"
        note="Change and File are fixed views, content-sized buttons outside the tab strip: they are not tabs, never shrink or scroll with them, and only compete with them for the selection. The strip holds the user's tabs, content-sized and capped at 160px, with scrolling and close menus that affect only tabs. The add control opens a tab in the conversation's browser: globe-plus while the strip is empty, a plain plus beside tabs. The new tab stands in the strip at once, selected, on about:blank, and its content says the browser is starting until its picture arrives; the gallery's browser takes about a second, as a Host takes a moment. A page the device closed keeps its tab, which says so and offers Close tab and Reload; a request the Host refuses shows its message with Retry. While a view connects the content only says that it waits. Resizing keeps the previous picture's aspect ratio until the browser supplies a frame at the new size; the old picture is never stretched to the new viewport. The viewport control is a square icon button, a computer or a phone, as far from the address as the navigation group is; its menu rows carry the same icons. A page tab opens only from an expose, with the expose glyph and the exposed address as its name, and frames its page in a sandbox: Refresh reloads it, the trailing control opens it in an ordinary browser tab, and Back and Forward stay unavailable because a framed page keeps its history to itself. Change groups its added/removed counts with a 2px gap and shows uncommitted totals and returns to Uncommitted when clicked; file pills still open retained edits there. File uses a Lucide outline icon until a file is selected, then its file-type icon. Every tab content puts its address row immediately below the strip; the same divider as File and Change separates it from the page. Browser and File navigation buttons have no extra gap between them; both address bars leave 12px after the navigation group. A tab the user just made opens with its address focused and selected, waiting for where to go. A click into an address selects it whole, a second click places the caret, and Enter submits it and lets the field go, so keys reach the page again."
      >
        <div class="grid gap-6 md:grid-cols-2">
          <GallerySpecimen variant="tabs" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <WorkPanel
                class="w-full"
                :views="exhibitWork.views.value" :panel="exhibitWork.panel.value" :kinds="exhibitWork.kinds"
                :workspace="workspace"
                @select="exhibitWork.select"
                @add-tab="exhibitWork.add" @update-tab="exhibitWork.update" @close-tabs="exhibitWork.closeTabs"
                @show-change="exhibitWork.showChange"
                @open="exhibitWork.open"
                @back="exhibitWork.back"
                @forward="exhibitWork.forward"
                @close="exhibitWork.reset"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="tabs · the conversation browser's tabs and a page an expose opened · live" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <WorkPanel
                class="w-full"
                :views="browserWork.views.value" :panel="browserWork.panel.value" :kinds="browserWork.kinds"
                :workspace="workspace"
                @select="browserWork.select"
                @add-tab="browserWork.add" @update-tab="browserWork.update" @close-tabs="browserWork.closeTabs"
                @show-change="browserWork.showChange"
                @open="browserWork.open"
                @back="browserWork.back"
                @forward="browserWork.forward"
              />
            </div>
            <!-- What the agent's close, or a browser that ended, does to the tab being shown. -->
            <div class="mt-2 flex items-center gap-2 text-[12px] text-fg-muted">
              <Button size="sm" :disabled="shownBrowserTab === null" @click="closeOnDevice">Close the page on the device</Button>
              <span>the shown browser tab stays, and says so</span>
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="File view"
        note="A file of the workspace: the path as crumbs from the workspace root, the file itself, and the workspace tree beside it with the file selected. Text opens read-only in the code editor, colored by its language, with folding and the first lines of the enclosing blocks kept at the top while scrolling. Mod-f in the text opens a find bar below it: the query with its match count, Match case, Match whole word and Use regular expression, and Previous and Next, which Shift+Enter and Enter in the field also do. Typing selects the first match from the selection on, the count shows ? while the selection is on no match, a count past 9999 stops there with a +, and the scrollbar marks every match counted until Escape or Close shuts the bar. An image fits the pane without being enlarged, over a checkerboard where it is transparent, and a click shows it at its actual size; video and audio play in the browser's own player and PDF in its own viewer, each with its pixel size and file size under it. Markdown renders like a repository file on GitHub: its HTML sanitized, so the script and the handler at the end of the README never run, its math and code rendered, its front matter a YAML block, and its links opening files here, scrolling to headings, or leaving for the web. Markdown and SVG switch between Preview and Source. A file that is neither text nor previewable is a card with its facts and Download, and every file has Download in the header. A crumb opens a menu of what lies beside it, directories unfolding into their own; a file picked there, clicked in the tree or linked from a document replaces the one shown, and Back and Forward before the crumbs walk the files shown. A click on the crumb row anywhere but a crumb turns it into a text field with the path, a relative one starting from the workspace: Enter opens a file, or finds a folder in the tree, unfolding down to it and selecting it until another file opens; a folder outside the workspace says the tree shows the workspace only. A right-click in the tree downloads a file, or uploads into a folder, or into the workspace from the empty space; the uploads list under the tree, moving here at a pace slow enough to watch. The control at the end of the crumb row hides and shows the tree. A view that would keep less than 320px beside the tree hides it by itself; the control then shows the tree over the file, and the control, a click beside the tree or a file picked in it puts it away. The tree docks again once the view is wide enough, and its divider stops where the file would get narrower than that; the narrow frame resizes from its corner. Reads carry the fixture's latency, so the text and each directory show their loading state first."
      >
        <div class="flex flex-wrap gap-1">
          <Button v-for="file in previewFiles" :key="file" size="sm" @click="showInFileView(`${workspace.root}/${file}`)">{{ file }}</Button>
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
        <GalleryFindBar />
      </GallerySection>
      <GallerySection
        title="Change view"
        note="Diffs from one of two sources, the switch in the header picks. A changed image, video, audio file or PDF shows its committed version beside the working tree's instead, each with its sizes, a new file only the second; a binary file with no preview shows a card per side with Download, and a committed version over 8 MiB says it is too large, with no Download. Markdown and SVG switch between the text diff and Preview, which renders both sides, labeled Committed and Working tree, or Before and After in Conversation. Uncommitted is the working tree against the last commit: the diff of the selected file beside the tree of the files git status lists, each ending its row with its line counts and the letter VS Code's Git marks it with, by VS Code's own rules from git's two status letters: U untracked, A added, M modified, D deleted (its name struck through), R renamed, T type changed, ! in conflict, in VS Code's colors and with VS Code's words as the tooltip; where git has a letter for both the index and the working tree, the working tree's shows, so a staged new file edited again is M. The fixtures hold every mark. The files and lines are summed up in the tree's caption. Conversation shows only the file picked under a shell call, without a file tree or a list source. It shows that file’s retained edits, with a segment control when other calls wrote between them. Missing contents leave the diff blank. Back and Forward walk what the view has shown, across modes. The header also opens the selected file itself. Only Uncommitted offers a tree toggle, and in a narrow view its tree hides and shows over the diff the way the File view's does; its tree's caption lists the changes again, its control turning while the list is on its way. Picking a file opens Conversation; selecting the fixed Change section returns to Uncommitted; with nothing picked, Conversation says how to fill it. Under Uncommitted, a workspace outside a Git repository shows “Not a git repository.” without the file tree or its toggle. It keeps the last list when a listing failed, and says under the rows when the list was cut short. A host can name the workspace in place of its directory's name, as the product does for the Cloud's own session directory."
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
        v-model:message-edit="messageEdit"
        @retry="sessionFlow.resume()"
        @rename="session.title = $event"
        @save-scroll="(_id, state) => (session.scroll = state)"
        @abort-subagents="abortAgents"
        @abort-subagent="abortAgent"
        @abort-terminal="abortTerminal"
        @remove-queued="removeQueued"
        @send-queued="sendNow"
        @remove-pending-steer="deletePendingSteer"
        @interrupt-pending-steer="interruptPendingSteer"
      >
        <template #composer>
          <GalleryComposer
            v-model:message-edit="messageEdit"
            @submit-edit="submitEdit"
            placeholder="Ask Demi about the failing login test…"
            :running="session.phase === 'running'"
            :compacting="compacting"
            :archived="session.archived"
            @restore="session.archived = false"
            @send="sessionFlow.turn"
            @queue="queueDraft"
            @stop="sessionFlow.stop"
            @compact="compact"
          />
        </template>
      </ChatSession>
    </template>
  </div>
</template>

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
import WorkPanel from '@demicodes/web-ui/agent/WorkPanel.vue'
import { changeWorkTab, changeTabPath, workPanelTabs, addBrowserTab, closeBrowserTabs, findChangeWorkTab, goBackInTab, goForwardInTab, showChangeInTab, showCallEdit, showFileInTab, type BrowserPage, type BrowserWorkTab, type ChangeWorkTab, type WorkTab } from '@demicodes/web-ui/agent/work-panel'
import { callChangeSource, type CallEditSelection, type ChangeMode, type ChangeSources } from '@demicodes/web-ui/files/changes'
import ChangeView from '@demicodes/web-ui/files/ChangeView.vue'
import FileView from '@demicodes/web-ui/files/FileView.vue'
import { createGalleryChangeSet, createGalleryWorkspace } from '../fixtures/workspace'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import { ASIDE_WIDTH, SIDEBAR_WIDTH } from '@demicodes/web-ui/sidebar/sidebar-width'
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
import { t } from '@demicodes/web-ui/infra/i18n'
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
import { submitMessageEdit, type MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import { firstRunningTerminalId } from '@demicodes/web-ui/agent/terminals'
import type { ThinkingConfig, UserContentBlock } from '@demicodes/core'
import { createPendingSteerMessage } from '@demicodes/web-ui/agent/pending-steers'
import { composerAttachment, encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
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
  longUserText,
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
import GalleryTabBar from '../components/GalleryTabBar.vue'
import GalleryTabBarDrive from '../components/GalleryTabBarDrive.vue'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()
const historyModelBlocks = transcriptDemoBlocks()
const submissionError = ref<string | null>('Connection closed before confirmation')

const messageEdit = ref<MessageEditState | null>(null)
const editRevision = ref(0)
const compacting = ref(false)
const fullComposer = ref<{ setDraft: (text: string) => void }>()
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
const panelAsideWidth = ref<number>(ASIDE_WIDTH.default)
const panelAsideOpen = ref(true)
const panelProjects = ref(demoProjects())
const panelConversations = ref(demoConversations())
const panelActiveConversationId = ref<string | null>('c-login')
/** Host-owned selections for the work-panel specimens. */
function useWorkTabs(activeId: string | null, path = 'src/auth/cookie.ts') {
  const tabs = ref(workPanelTabs(path))
  const active = ref<string | null>(activeId)
  function addBrowser(page?: BrowserPage) {
    const next = addBrowserTab(tabs.value, page)
    tabs.value = next.tabs
    active.value = next.activeId
  }
  function closeTabs(ids: string[]) {
    const next = closeBrowserTabs(tabs.value, active.value, ids)
    tabs.value = next.tabs
    active.value = next.activeId
  }
  function updateBrowser(tab: BrowserWorkTab) {
    tabs.value = tabs.value.map((current) => current.id === tab.id ? tab : current)
  }
  function showChange(id: string, mode: ChangeMode, path: string | null, selection?: { call: ChangeWorkTab['call']; edit: number }) {
    tabs.value = showChangeInTab(tabs.value, id, mode, path, selection)
  }
  /** A file by workspace path, shown in the active tab in place. */
  function open(path: string) {
    const next = showFileInTab(tabs.value, path)
    tabs.value = next.tabs
    active.value = next.activeId
  }
  function back(id: string) {
    tabs.value = goBackInTab(tabs.value, id)
  }
  function forward(id: string) {
    tabs.value = goForwardInTab(tabs.value, id)
  }
  function selectEdit(selection: CallEditSelection) {
    const next = showCallEdit(tabs.value, selection)
    tabs.value = next.tabs
    active.value = next.activeId
  }
  function reset() {
    tabs.value = workPanelTabs(path)
    active.value = activeId
  }
  return { tabs, active, addBrowser, closeTabs, updateBrowser, open, showChange, selectEdit, back, forward, reset }
}
const workspace = createGalleryWorkspace()
const fileViewTree = ref(true)
const changeViewTree = ref(true)
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
const exhibitWork = useWorkTabs('file')
const editWork = useWorkTabs('change')
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
const changeEmpty = useChangeTab('conversation', null, { conversation: null, uncommitted: workspace.changes })
const changeNoRepository = useChangeTab('uncommitted', null, {
  conversation: null,
  uncommitted: createGalleryChangeSet(200, { unavailable: 'no-repository' }),
})
const changeStale = useChangeTab('uncommitted', 'src/auth/cookie.ts', {
  conversation: null,
  uncommitted: createGalleryChangeSet(200, { truncated: true, failure: 'The device is offline.' }),
})
// One empty tab with the globe, then one opened the way an expose row opens it, with the expose glyph.
const browserWork = useWorkTabs('change', '')
browserWork.addBrowser()
browserWork.addBrowser({
  url: `data:text/html,${encodeURIComponent('<body style="font:14px system-ui;padding:24px"><h1>Dev server</h1><p>A page shown in the tab\'s sandboxed frame.</p><a href="https://example.com" target="_blank">A link that opens a popup</a></body>')}`,
  title: '127.0.0.1:5173',
  expose: true,
})
let nextQueue = 3
let nextSent = 1

// The Session view is the product's ChatSession over a scripted runtime; Turns and Stream replay one flow each.
const sessionFlow = useTurnFlow({
  id: 'gallery-session',
  title: 'Login test',
  blocks: transcriptDemoBlocks(),
  subagents: agents,
  terminals,
})
const session = sessionFlow.state
session.pendingSteers = [createPendingSteerMessage('pending-1', steerPrompt, session.blocks)]
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
const attachmentBubble = [
  {
    type: 'image' as const,
    source: {
      type: 'url' as const,
      url: demoImageUrl,
    },
  },
  {
    type: 'attachment' as const,
    name: 'login-fail.png',
    path: '/home/demi/.demi/attachments/demo/login-fail.png',
    mediaType: 'image/png',
    sizeBytes: 48211,
    sha256: 'demo-png',
  },
  {
    type: 'document' as const,
    source: {
      data: new Uint8Array(),
      mediaType: 'application/pdf',
      fileName: 'login-failure.pdf',
    },
  },
  {
    type: 'attachment' as const,
    name: 'login-failure.pdf',
    path: '/home/demi/.demi/attachments/demo/login-failure.pdf',
    mediaType: 'application/pdf',
    sizeBytes: 120334,
    sha256: 'demo-pdf',
  },
  {
    type: 'attachment' as const,
    name: 'ci.log',
    path: '/home/demi/.demi/attachments/demo/ci.log',
    mediaType: 'text/plain',
    sizeBytes: 2048,
    sha256: 'demo-log',
    snippet: pastedSnippet,
  },
  {
    type: 'reference' as const,
    reference: encodeRemoteReference(
      'zan-mbp',
      '/Users/zan/Projects/demi/package.json',
    ),
  },
  {
    type: 'text' as const,
    text: 'Failing log and the screenshot from CI.',
  },
]
const overflowBubble = [
  {
    type: 'text' as const,
    text: longUserText,
  },
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
const activityKinds: ActivityKind[] = ['requesting', 'connecting', 'resuming', 'retrying']
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
  const text = pending.content.find(
    (part): part is Extract<typeof part, { type: 'text' }> => part.type === 'text',
  )?.text
  if (!text) {
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
      content: [{ type: 'text', text }],
    },
  ]
  sessionFlow.turn(text)
}

function queueDraft(text: string): void {
  session.queue.push({
    id: `q${nextQueue++}`,
    text,
    content: [{ type: 'text', text }],
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
    createPendingSteerMessage(`gallery-${nextSent++}`, item.content, session.blocks),
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
        title="Tab bar"
        note="Session tabs and the conversation list. Tabs fit their icon and title up to 160px; longer titles truncate."
      >
        <GalleryTabBar />
      </GallerySection>
      <GallerySection
        title="Motion"
        note="Every motion the strip owns, in a narrow frame that overflows and one at the pane width. A tab opens by growing from nothing and closes by collapsing, while the New tab control follows the last tab until they scroll and then holds the right edge. Selecting or opening a tab that is cut off scrolls, with the same timing as the tab, until it shows whole and clear of the fade; closing scrolls back when the end comes into reach. Play all runs through the cases; the tabs, their close controls and their menus work too."
      >
        <GallerySpecimen variant="narrow · overflows" wide>
          <GalleryTabBarDrive width="32rem" />
        </GallerySpecimen>
        <GallerySpecimen variant="wide · pane width" wide>
          <GalleryTabBarDrive />
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'composer'">
      <GallerySection
        title="Composer"
        note="Idle through Fast Mode, local upload phases, a remote host file as the same tile, and queue. One send; a running turn queues. An unavailable last model keeps the chip, warns, and blocks send. No usable model and an archived conversation both replace the input with the same snackbar: a line on the left, Configure models or Restore conversation on the right."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen
            variant="idle · empty"
            wide
          >
            <GalleryComposer placeholder="Ask Demi…" />
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
            variant="attachments · ready"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              draft="Failing log and the screenshot from CI."
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
              draft="Wait for the screenshot."
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
            variant="drop · anywhere on the composer"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              dropping
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="paste · a long text becomes pasted-text.txt"
            wide
          >
            <GalleryComposer
              placeholder="Paste 2000+ characters or 40+ lines here…"
              draft="Summarize this log."
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
        note="User, attachments, overflow, pending steer, queued, and stuck."
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
            variant="attachments"
            wide
          >
            <div class="gallery-frame gallery-user-frame bg-surface">
              <UserBlock :content="attachmentBubble" />
            </div>
          </GallerySpecimen>
          <GallerySpecimen
            variant="overflow"
            wide
          >
            <div class="gallery-frame gallery-user-frame bg-surface">
              <div class="gallery-user-overflow">
                <UserBlock :content="overflowBubble" />
              </div>
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
                variant="steer"
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
        note="Requesting shows elapsed time after one second; incoming blocks roll into the same row."
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
          :tabs="editWork.tabs.value" :active-id="editWork.active.value"
          :workspace="workspace" :read-call-change="readCallChange"
          @select="editWork.active.value = $event" @add-browser="editWork.addBrowser" @close-tabs="editWork.closeTabs" @update-browser="editWork.updateBrowser" @show-change="editWork.showChange" @open="editWork.open"
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
        note="The shared product page, with local fixture state and handlers."
      >
        <GalleryConnectedSession />
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
                text="Review the attached notes."
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
        note="The app frame with the work panel open on the right: the sidebar, the session, and the panel are siblings, each side pane behind its own divider. The header's panel control opens it and the panel's fold control closes it, using the same 28px button, 14px icon, 12px right inset, tooltip and hover treatment as the conversation’s Open panel control; drag or double-click the divider on its left; Reset panel restores the initial selections."
      >
        <GallerySpecimen variant="frame · live" wide>
          <div class="gallery-frame flex h-[44rem] w-full overflow-hidden">
            <SidebarLayout
              v-model:width="panelSidebarWidth"
              v-model:aside-open="panelAsideOpen"
              v-model:aside-width="panelAsideWidth"
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
                @open-aside="panelAsideOpen = true"
                @retry="sessionFlow.resume()"
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
                    :conversation-id="session.id"
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
                  :tabs="panelWork.tabs.value"
                  :active-id="panelWork.active.value"
                  :workspace="workspace"
                  @select="(id) => (panelWork.active.value = id)"
                  @add-browser="panelWork.addBrowser" @close-tabs="panelWork.closeTabs" @update-browser="panelWork.updateBrowser"
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
          <span class="self-center font-mono text-[11px] text-fg-faint">panel {{ panelAsideWidth }}px · {{ ASIDE_WIDTH.min }}–{{ ASIDE_WIDTH.max }}</span>
        </div>
      </GallerySection>
      <GallerySection
        title="Work panel"
        note="Change and File are content-sized buttons outside the browser tab strip. They never shrink or scroll with browser tabs; only the browser strip uses the remaining width. The add button uses globe-plus when no browser tabs exist and a regular plus otherwise; it adds browser tabs, using the same content-sized tabs capped at 160px, scrolling and close menus. Browser close menus affect only browser tabs. Change groups its added/removed counts with a 2px gap and shows uncommitted totals and returns to Uncommitted when clicked; file pills still open retained edits there. File uses a Lucide outline icon until a file is selected, then its file-type icon. Browser tabs use a globe. Selecting one shows Back, Forward, Refresh and its own address draft immediately below the strip; the same divider as File and Change separates the address row from page content. Browser and File navigation buttons have no extra gap between them; both address bars leave 12px after the navigation group. A browser tab frames its page in a sandbox: Refresh reloads it, the trailing control opens it in an ordinary browser tab, and Back and Forward stay unavailable because a framed page keeps its history to itself."
      >
        <div class="grid gap-6 md:grid-cols-2">
          <GallerySpecimen variant="tabs" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <WorkPanel
                class="w-full"
                :tabs="exhibitWork.tabs.value"
                :active-id="exhibitWork.active.value"
                :workspace="workspace"
                @select="(id) => (exhibitWork.active.value = id)"
                @add-browser="exhibitWork.addBrowser" @close-tabs="exhibitWork.closeTabs" @update-browser="exhibitWork.updateBrowser"
                @show-change="exhibitWork.showChange"
                @open="exhibitWork.open"
                @back="exhibitWork.back"
                @forward="exhibitWork.forward"
                @close="exhibitWork.reset"
              />
            </div>
          </GallerySpecimen>
          <GallerySpecimen variant="browser tabs · an empty tab and a page in the sandboxed frame" wide>
            <div class="gallery-frame flex h-[24rem] overflow-hidden">
              <WorkPanel
                class="w-full"
                :tabs="browserWork.tabs.value"
                :active-id="browserWork.active.value"
                :workspace="workspace"
                @select="browserWork.active.value = $event"
                @add-browser="browserWork.addBrowser" @close-tabs="browserWork.closeTabs" @update-browser="browserWork.updateBrowser"
                @show-change="browserWork.showChange"
                @open="browserWork.open"
                @back="browserWork.back"
                @forward="browserWork.forward"
              />
            </div>
          </GallerySpecimen>
        </div>
      </GallerySection>
      <GallerySection
        title="File view"
        note="A file of the workspace: the path as crumbs from the workspace root, the highlighted text, and the workspace tree beside it with the file selected. A crumb opens a menu of what lies beside it, directories unfolding into their own; a file picked there, or clicked in the tree, replaces the one shown, and Back and Forward before the crumbs walk the files shown. The control at the end of the crumb row hides and shows the tree. Reads carry the fixture's latency, so the text and each directory show their loading state first."
      >
        <GallerySpecimen variant="cookie.ts · live" wide>
          <div class="gallery-frame flex h-[40rem] overflow-hidden">
            <FileView
              class="w-full"
              v-model:tree="fileViewTree"
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
      </GallerySection>
      <GallerySection
        title="Change view"
        note="Diffs from one of two sources, the switch in the header picks. Uncommitted is the working tree against the last commit: the diff of the selected file beside the tree of changed files with the kind of each change (a green dot for a new file, a struck name for a deleted one) and its line counts, the files and lines summed up in the tree's caption. Conversation shows only the file picked under a shell call, without a file tree or a list source. It shows that file’s retained edits, with a segment control when other calls wrote between them. Missing contents leave the diff blank. Either way the header names the file shown with its counts. Back and Forward walk what the view has shown, across modes. The header also opens the selected file itself. Only Uncommitted offers a tree toggle; its tree's caption lists the changes again, its control turning while the list is on its way. Picking a file opens Conversation; selecting the fixed Change section returns to Uncommitted; with nothing picked, Conversation says how to fill it. Under Uncommitted, a workspace outside a Git repository shows “Not a git repository.” without the file tree or its toggle. It keeps the last list when a listing failed, and says under the rows when the list was cut short. A host can name the workspace in place of its directory's name, as the product does for the Cloud's own session directory."
      >
        <GallerySpecimen
          v-for="specimen in [
            { variant: 'uncommitted · live', work: changeUncommitted, rootName: undefined },
            { variant: 'uncommitted · not a repository, named Workspace', work: changeNoRepository, rootName: 'Workspace' },
            { variant: 'uncommitted · listing failed, cut short', work: changeStale, rootName: undefined },
            { variant: 'conversation · picked', work: changePicked, rootName: undefined },
            { variant: 'conversation · nothing picked', work: changeEmpty, rootName: undefined },
          ]"
          :key="specimen.variant"
          :variant="specimen.variant"
          wide
        >
          <div class="gallery-frame flex h-[40rem] overflow-hidden">
            <ChangeView
              class="w-full"
              v-model:tree="changeViewTree"
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
        :edit-version="editVersion"
        v-model:message-edit="messageEdit"
        @retry="sessionFlow.resume()"
        @archive="session.archived = true"
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
            ref="fullComposer"
            v-model:message-edit="messageEdit"
            @submit-edit="submitEdit"
            placeholder="Ask Demi about the failing login test…"
            :conversation-id="session.id"
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

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
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
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
import GalleryConnectedSession from '../components/GalleryConnectedSession.vue'
import GalleryMessageEditing from '../components/GalleryMessageEditing.vue'
import GalleryAssistantMessages from '../components/GalleryAssistantMessages.vue'
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
        note="Session tabs and the conversation list."
      >
        <GalleryTabBar />
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
        note="The model dropdown aligns to the trigger’s right edge, extending to the left."
      >
        <div class="specimen-stack">
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
        note="Fork completed messages while the source runs. Preview success, pending creation, and a retry after failure."
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

    <template v-if="view === 'session'">
      <ChatSession
        :conversation="session"
        has-provider
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

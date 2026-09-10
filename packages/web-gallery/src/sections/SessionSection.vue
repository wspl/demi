<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { Play, RotateCw } from '@lucide/vue'
import ThinkingBlock from '@demicodes/web-ui/agent/blocks/ThinkingBlock.vue'
import ErrorBlock from '@demicodes/web-ui/agent/blocks/ErrorBlock.vue'
import ToolShellBlock from '@demicodes/web-ui/agent/blocks/ToolShellBlock.vue'
import { parseToolInput } from '@demicodes/web-ui/agent/block-helpers'
import LoadingBlock from '@demicodes/web-ui/agent/blocks/LoadingBlock.vue'
import UserBlock from '@demicodes/web-ui/agent/blocks/UserBlock.vue'
import ModelMenu from '@demicodes/web-ui/agent/ModelMenu.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import PendingSubmission from '@demicodes/web-ui/agent/PendingSubmission.vue'
import { RestoreSweep } from '@demicodes/web-ui/agent/session-restore'
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
import { useSessionPanels } from '@demicodes/web-ui/agent/useSessionPanels'
import GalleryConnectedSession from '../components/GalleryConnectedSession.vue'
import GalleryMessageEditing from '../components/GalleryMessageEditing.vue'
import { beginMessageEdit, submitMessageEdit, type MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import { firstRunningTerminalId } from '@demicodes/web-ui/agent/terminals'
import type { Block, ThinkingConfig, UserContentBlock } from '@demicodes/core'
import type { PendingSteerRenderBlock } from '@demicodes/web-ui/agent/pending-steers'
import { queuedMessagesToRenderBlocks } from '@demicodes/web-ui/agent/queued-messages'
import { encodeRemoteReference } from '@demicodes/web-ui/agent/message-input/attachments'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Button from '@demicodes/web-ui/ui/Button.vue'
import ActivitySlot from '../components/ActivitySlot.vue'
import {
  demoImageUrl,
  demoModel,
  shellTool,
  thinkingText,
  longUserText,
  pendingSteerDemo,
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
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from '../components/GalleryComposer.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySessionPane from '../components/GallerySessionPane.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import GalleryTabBar from '../components/GalleryTabBar.vue'
import GalleryTranscript from '../components/GalleryTranscript.vue'
import { useGalleryView } from '../gallery-views'

const { view } = useGalleryView()
const submissionError = ref<string | null>('Connection closed before confirmation')

const hiddenIds = ref(new Set<string>())
const sessionBase = ref(transcriptDemoBlocks())
const messageEdit = ref<MessageEditState | null>(null)
const editRevision = ref(0)
const extras = ref<Block[]>([])
// Pending steers sit after every transcript block, like AgentMessageList orders them.
const pendingSteers = ref<PendingSteerRenderBlock[]>([pendingSteerDemo])
const compacting = ref(false)
const queue = ref([
  {
    id: 'q1',
    text: 'Also add a case for the expired cookie.',
  },
  {
    id: 'q2',
    text: 'Keep the light-mode screenshot in the same PR.',
  },
])
const fullPane = ref<{ scrollToEnd: () => void }>()
const fullComposer = ref<{ setDraft: (text: string) => void }>()
const turnPane = ref<{ scrollToEnd: () => void }>()
const agents = reactive(gallerySubagents())
const terminals = galleryTerminals()
const finishedOnly = agents.filter((agent) => agent.phase !== 'running')
const { activeSubagentId, activeTerminalId, toggleAgents, toggleTerminals } =
  useSessionPanels(
    () => agents,
    () => terminals,
  )
const exhibitAgentId = ref<string | null>(
  runningSubagents(agents)[0]?.id ?? agents[0]?.id ?? null,
)
const exhibitTerminalId = ref<string | null>(firstRunningTerminalId(terminals))
const fillPane = computed(() => view.value === 'session')

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

const sessionFlow = useTurnFlow()
const streamFlow = useTurnFlow()
const turnFlow = useTurnFlow()
const {
  blocks: sessionFlowBlocks,
  slot: sessionSlot,
  endedAtById: sessionEndedAt,
  streamingThinkingId: sessionStreamingId,
  streamingTextId: sessionTextId,
  running: sessionRunning,
  play: playSession,
  stop: stopSession,
} = sessionFlow
const {
  blocks: streamBlocks,
  endedAtById: streamEndedAt,
  streamingThinkingId: streamThinkingId,
  streamingTextId: streamTextId,
} = streamFlow

function playStream(): void {
  streamFlow.play('stream')
}
const {
  blocks: turnBlocks,
  slot: turnSlot,
  endedAtById: turnEndedAt,
  streamingThinkingId: turnStreamingId,
  streamingTextId: turnTextId,
  play: playTurn,
} = turnFlow

const sessionBlocks = computed(() => [
  ...sessionBase.value.filter((block) => !hiddenIds.value.has(block.id)),
  ...extras.value.filter((block) => !hiddenIds.value.has(block.id)),
  ...sessionFlowBlocks.value,
  ...pendingSteers.value,
  ...queuedMessagesToRenderBlocks(queue.value),
])

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
const attachmentBubble = [
  {
    type: 'image' as const,
    source: {
      type: 'url' as const,
      url: demoImageUrl,
    },
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
const functionalError = ref(true)
const functionalThinkingStartedAt = new Date().toISOString()
const functionalThinkingEndedAt = new Date(
  Date.parse(functionalThinkingStartedAt) + 8_000,
).toISOString()

function hideBlock(id: string): void {
  hiddenIds.value = new Set(hiddenIds.value).add(id)
}

function takePendingSteer(
  pendingSteerId: string,
): PendingSteerRenderBlock | undefined {
  const block = pendingSteers.value.find(
    (candidate) => candidate.pendingSteerId === pendingSteerId,
  )
  pendingSteers.value = pendingSteers.value.filter(
    (candidate) => candidate !== block,
  )
  return block
}

function deletePendingSteer(pendingSteerId: string): void {
  takePendingSteer(pendingSteerId)
}

function interruptPendingSteer(pendingSteerId: string): void {
  const block = takePendingSteer(pendingSteerId)
  if (!block) {
    return
  }
  const text = block.content.find(
    (
      part,
    ): part is Extract<
      typeof part,
      {
        type: 'text'
      }
    > => part.type === 'text',
  )?.text
  if (text) {
    extras.value = [
      ...extras.value,
      {
        type: 'steer',
        id: `steer-sent-${nextSent++}`,
        turnId: 'turn-gallery',
        createdAt: new Date().toISOString(),
        model: demoModel,
        content: [
          {
            type: 'text',
            text,
          },
        ],
      },
    ]
    playSession('turn', text)
  }
}

function send(text: string): void {
  playSession('turn', text)
}

function queueDraft(text: string): void {
  queue.value.push({
    id: `q${nextQueue++}`,
    text,
  })
  fullPane.value?.scrollToEnd()
}

function removeQueued(id: string): void {
  queue.value = queue.value.filter((entry) => entry.id !== id)
}

function sendNow(id: string): void {
  const item = queue.value.find((entry) => entry.id === id)
  if (!item) {
    return
  }
  queue.value = queue.value.filter((entry) => entry.id !== id)
  pendingSteers.value = [
    ...pendingSteers.value,
    {
      type: 'pending_steer',
      id: `pending-steer:gallery-${nextSent}`,
      pendingSteerId: `gallery-${nextSent++}`,
      content: [
        {
          type: 'text',
          text: item.text,
        },
      ],
    },
  ]
  fullPane.value?.scrollToEnd()
}

function editUser(id: string): void {
  const block = sessionBlocks.value.find((part) => part.id === id)
  if (!block || block.type !== 'user' || messageEdit.value) {
    return
  }
  messageEdit.value = beginMessageEdit(block, { epoch: 'gallery-session', revision: editRevision.value })
}

async function submitEdit(): Promise<void> {
  await submitMessageEdit({
    get: () => messageEdit.value,
    set: (state) => { messageEdit.value = state },
    send: async (request) => {
      const blocks = [...sessionBase.value, ...extras.value, ...sessionFlowBlocks.value]
        .filter((block) => !hiddenIds.value.has(block.id)) as Block[]
      const index = blocks.findIndex((block) => block.id === request.targetBlockId)
      if (index < 0) {
        throw new Error('Message not found')
      }
      sessionBase.value = [
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
      extras.value = []
      sessionFlowBlocks.value = []
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

watch([sessionFlowBlocks, sessionSlot, queue], () => {
  fullPane.value?.scrollToEnd()
})

watch([turnBlocks, turnSlot], () => {
  turnPane.value?.scrollToEnd()
})

const sessionRestore = new RestoreSweep()
const composerArchived = ref(true)
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

onBeforeUnmount(() => sessionRestore.stop())
function abortAgents() {
  for (const agent of agents) {
    if (agent.phase === 'running') {
      agent.phase = 'aborted'
      agent.endedAt = new Date().toISOString()
    }
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
                  destination: 'workspace',
                  phase: 'ready',
                },
                {
                  id: 'png',
                  name: 'login-fail.png',
                  src: demoImageUrl,
                  destination: 'message',
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
                  destination: 'message',
                  phase: 'uploading',
                  progress: 0.42,
                },
                {
                  id: 'pdf-up',
                  name: 'spec.pdf',
                  destination: 'workspace',
                  phase: 'uploading',
                  progress: 0.68,
                },
              ]"
            />
          </GallerySpecimen>
          <GallerySpecimen
            variant="drop"
            wide
          >
            <GalleryComposer
              placeholder="Ask Demi…"
              dropping
            />
          </GallerySpecimen>
          <GallerySpecimen variant="local draft · files upload on first send" wide>
            <GalleryComposer
              placeholder="Ask Demi…"
              draft="Review this file."
              :attachments="[{ id: 'staged-file', name: 'notes.txt', destination: 'workspace', phase: 'staged' }]"
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
        note="28px capsule with status dots. Running and Agents open their windows; a second click closes them."
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
          <GallerySpecimen variant="agents · none running">
            <AgentsChip :agents="finishedOnly" />
          </GallerySpecimen>
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'blocks'">
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
                v-model:open="functionalError"
                message="Provider aborted after 3 retries."
                code="rate_limit"
                :diagnostics="{ source: 'http', httpStatus: 429 }"
              />
            </GallerySpecimen>
          </div>
        </div>
      </GallerySection>

      <GallerySection
        title="LoadingBlock"
        note="Requesting row before the first block."
      >
        <GallerySpecimen
          variant="requesting"
          wide
        >
          <div class="gallery-frame gallery-activity-frame bg-surface">
            <LoadingBlock />
          </div>
        </GallerySpecimen>
      </GallerySection>

      <GallerySection
        title="ActivitySlot"
        note="Connecting, resuming, retrying, and requesting."
      >
        <GallerySpecimen
          variant="connecting"
          wide
        >
          <div class="gallery-frame gallery-activity-frame bg-surface">
            <ActivitySlot
              kind="connecting"
              label="Connecting"
            />
          </div>
        </GallerySpecimen>
      </GallerySection>
    </template>

    <template v-if="view === 'turns'">
      <GallerySessionPane
        ref="turnPane"
        label="Turn"
      >
        <GalleryTranscript
          :blocks="turnBlocks"
          :streaming-thinking-id="turnStreamingId"
          :streaming-text-id="turnTextId"
          :ended-at-by-id="turnEndedAt"
          :activity="turnSlot"
        />
        <template #dock="{ showScrollToBottom, scrollToEnd }">
          <SessionDock
            :show-scroll-to-bottom="showScrollToBottom"
            @scroll-to-bottom="scrollToEnd"
          >
            <template #chips>
              <SessionDockChip @click="playTurn('turn')">
                <RotateCw :size="ICON_PX.in28" />
                Replay
              </SessionDockChip>
              <SessionDockChip @click="playTurn('resume')">
                <Play :size="ICON_PX.in28" />
                Resume
              </SessionDockChip>
              <SessionDockChip @click="playTurn('retry')"> Retry </SessionDockChip>
              <SessionDockChip @click="playTurn('connect')">
                Connect
              </SessionDockChip>
            </template>
          </SessionDock>
        </template>
      </GallerySessionPane>

      <GallerySection
        title="Stream"
        note="Thinking then reply, same reveal."
      >
        <div class="mb-3">
          <Button
            variant="ghost"
            size="sm"
            @click="playStream"
            >Replay</Button
          >
        </div>
        <div class="gallery-frame gallery-block-frame-y bg-surface">
          <GalleryTranscript
            :blocks="streamBlocks"
            :streaming-thinking-id="streamThinkingId"
            :streaming-text-id="streamTextId"
            :ended-at-by-id="streamEndedAt"
          />
        </div>
      </GallerySection>
    </template>

    <template v-if="view === 'windows'">
      <GallerySection
        title="Agents"
        note="Half-session inspect. Tabs are running children; Completed on the right is a searchable finished list."
      >
        <div class="relative h-[24rem] min-h-0">
          <SubagentPanel
            v-model:active-id="exhibitAgentId"
            :agents="agents"
            @abort="abortAgents"
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
        title="Session load"
        note="History arriving, a dropped socket, and a failed restore. A dropped socket uses the Requesting tail row labeled Connecting. An unconfirmed send keeps its user message and retries with the same ID. New conversation opens a local draft immediately."
      >
        <div class="specimen-stack specimen-stack-loose">
          <GallerySpecimen variant="unconfirmed send · retry the same message" wide>
            <div class="gallery-frame bg-surface">
              <PendingSubmission
                id="pending-example"
                text="Review the attached notes."
                :file-names="['notes.txt']"
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
          <GallerySpecimen
            variant="reconnecting"
            wide
          >
            <div class="gallery-frame gallery-activity-frame bg-surface">
              <LoadingBlock :label="t('agent.block.connecting')" />
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
                <LoadingBlock :label="t('agent.block.connecting')" />
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
      <GallerySessionPane
        ref="fullPane"
        fill
      >
        <GalleryTranscript
          :blocks="sessionBlocks"
          :streaming-thinking-id="sessionStreamingId ?? 'thinking-streaming'"
          :streaming-text-id="sessionTextId"
          :ended-at-by-id="sessionEndedAt"
          :activity="sessionSlot"
          :editable="!sessionRunning && !queue.length && !pendingSteers.length && !messageEdit"
          :edit-target-id="messageEdit?.request.targetBlockId"
          @delete-pending-steer="deletePendingSteer"
          @interrupt-pending-steer="interruptPendingSteer"
          @delete-queued="removeQueued"
          @send-queued="sendNow"
          @edit-user="editUser"
        />
        <template #dock="{ showScrollToBottom, scrollToEnd }">
          <SessionDock
            :show-scroll-to-bottom="showScrollToBottom"
            @scroll-to-bottom="scrollToEnd"
          >
            <template #chips>
              <SessionDockChip @click="playSession('resume')">
                <Play :size="ICON_PX.in28" />
                Resume
              </SessionDockChip>
              <TerminalChip
                :terminals="terminals"
                :open="activeTerminalId != null"
                @open="toggleTerminals"
              />
              <AgentsChip
                :agents="agents"
                :open="activeSubagentId != null"
                @open="toggleAgents"
              />
            </template>
            <GalleryComposer
              ref="fullComposer"
              v-model:message-edit="messageEdit"
              @submit-edit="submitEdit"
              placeholder="Ask Demi about the failing login test…"
              conversation-id="demo"
              :running="sessionRunning"
              :compacting="compacting"
              @send="send"
              @queue="queueDraft"
              @stop="stopSession"
              @compact="compact"
            />
          </SessionDock>
        </template>
        <template #panel>
          <SubagentPanel
            v-model:active-id="activeSubagentId"
            :agents="agents"
            @abort="abortAgents"
          />
          <TerminalPanel
            v-model:active-id="activeTerminalId"
            :terminals="terminals"
          />
        </template>
      </GallerySessionPane>
    </template>
  </div>
</template>

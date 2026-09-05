<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { Brain, Play, RotateCw, Terminal } from '@lucide/vue'
import FunctionalBlock from '@demicodes/web-ui/agent/blocks/FunctionalBlock.vue'
import LoadingBlock from '@demicodes/web-ui/agent/blocks/LoadingBlock.vue'
import UserBlock from '@demicodes/web-ui/agent/blocks/UserBlock.vue'
import ModelMenu from '@demicodes/web-ui/agent/ModelMenu.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import type { ThinkingConfig } from '@demicodes/core'
import type { MessageListBlock } from '@demicodes/web-ui/agent/pending-steers'
import { queuedMessagesToRenderBlocks } from '@demicodes/web-ui/agent/queued-messages'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Button from '@demicodes/web-ui/ui/Button.vue'
import ActivitySlot from '../components/ActivitySlot.vue'
import ExploratoryMark from '../components/ExploratoryMark.vue'
import {
  demoImageUrl,
  demoModel,
  longUserText,
  steerPrompt,
  transcriptDemoBlocks,
} from '../fixtures/blocks'
import { demoModels, demoProviders, mediumThinking, usageAt } from '../fixtures/catalog'
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from '../components/GalleryComposer.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySessionPane from '../components/GallerySessionPane.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import GalleryTabBar from '../components/GalleryTabBar.vue'
import GalleryTranscript from '../components/GalleryTranscript.vue'

const hiddenIds = ref(new Set<string>())
const extras = ref<MessageListBlock[]>([])
const compacting = ref(false)
const queue = ref([
  { id: 'q1', text: 'Also add a case for the expired cookie.' },
  { id: 'q2', text: 'Keep the light-mode screenshot in the same PR.' },
])
const fullPane = ref<{ scrollToEnd: () => void }>()
const turnPane = ref<{ scrollToEnd: () => void }>()
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
  ...transcriptDemoBlocks().filter((block) => !hiddenIds.value.has(block.id)),
  ...extras.value.filter((block) => !hiddenIds.value.has(block.id)),
  ...sessionFlowBlocks.value,
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
  { type: 'image' as const, source: { type: 'url' as const, url: demoImageUrl } },
  { type: 'document' as const, source: { data: new Uint8Array(), mediaType: 'application/pdf', fileName: 'login-failure.pdf' } },
  { type: 'text' as const, text: 'Failing log and the screenshot from CI.' },
]
const overflowBubble = [{ type: 'text' as const, text: longUserText }]
const userBubble = [{ type: 'text' as const, text: 'The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.' }]
const queuedBubble = [{ type: 'text' as const, text: 'Also add a case for the expired cookie.' }]
const stuckBubble = [{ type: 'text' as const, text: 'Do not touch the cookie helper. Only fix the assertion.' }]
const pendingSteerShown = ref(true)
const queuedShown = ref(true)
const functionalCollapsed = ref(false)
const functionalExpanded = ref(true)
const functionalTool = ref(false)
const functionalError = ref(false)

function hideBlock(id: string): void {
  hiddenIds.value = new Set(hiddenIds.value).add(id)
}

function deletePendingSteer(pendingSteerId: string): void {
  const block = sessionBlocks.value.find(
    (candidate): candidate is Extract<MessageListBlock, { type: 'pending_steer' }> =>
      candidate.type === 'pending_steer' && candidate.pendingSteerId === pendingSteerId,
  )
  if (block) hideBlock(block.id)
}

function interruptPendingSteer(pendingSteerId: string): void {
  const block = sessionBlocks.value.find(
    (candidate): candidate is Extract<MessageListBlock, { type: 'pending_steer' }> =>
      candidate.type === 'pending_steer' && candidate.pendingSteerId === pendingSteerId,
  )
  if (!block) return
  hideBlock(block.id)
  const text = block.content.find((part): part is Extract<typeof part, { type: 'text' }> => part.type === 'text')?.text
  if (text) {
    extras.value = [
      ...extras.value,
      {
        type: 'steer',
        id: `steer-sent-${nextSent++}`,
        turnId: 'turn-gallery',
        createdAt: new Date().toISOString(),
        model: demoModel,
        content: [{ type: 'text', text }],
      },
    ]
    playSession('turn', text)
  }
}

function send(text: string): void {
  playSession('turn', text)
}

function queueDraft(text: string): void {
  queue.value.push({ id: `q${nextQueue++}`, text })
  fullPane.value?.scrollToEnd()
}

function removeQueued(id: string): void {
  queue.value = queue.value.filter((entry) => entry.id !== id)
}

function sendNow(id: string): void {
  const item = queue.value.find((entry) => entry.id === id)
  if (!item) return
  queue.value = queue.value.filter((entry) => entry.id !== id)
  extras.value = [
    ...extras.value,
    {
      type: 'pending_steer',
      id: `pending-steer:gallery-${nextSent}`,
      pendingSteerId: `gallery-${nextSent++}`,
      content: [{ type: 'text', text: item.text }],
    },
  ]
  fullPane.value?.scrollToEnd()
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

onMounted(() => {
  playStream()
  playTurn('turn')
})
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection title="Tab bar" note="Session tabs and the conversation list.">
      <GalleryTabBar />
    </GallerySection>

    <GallerySection title="Composer" note="Idle through Fast Mode, attachments, and queue. One send; a running turn queues.">
      <div class="specimen-stack specimen-stack-loose">
        <GallerySpecimen variant="idle · empty" wide>
          <GalleryComposer placeholder="Ask Demi…" />
        </GallerySpecimen>
        <GallerySpecimen variant="focused" wide>
          <GalleryComposer placeholder="Ask Demi…" focused />
        </GallerySpecimen>
        <GallerySpecimen variant="send-ready" wide>
          <GalleryComposer placeholder="Ask Demi…" draft="The login test in packages/web/src/auth.test.ts is failing after the session cookie rename." />
        </GallerySpecimen>
        <GallerySpecimen variant="multiline" wide>
          <GalleryComposer
            placeholder="Ask Demi…"
            draft="The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.&#10;Keep the fix in that file."
          />
        </GallerySpecimen>
        <GallerySpecimen variant="attachments" wide>
          <GalleryComposer
            placeholder="Ask Demi…"
            draft="Failing log and the screenshot from CI."
            :attachments="[
              { name: 'login-failure.pdf' },
              { name: 'login-fail.png', src: demoImageUrl },
            ]"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="drop" wide>
          <GalleryComposer placeholder="Ask Demi…" dropping />
        </GallerySpecimen>
        <GallerySpecimen variant="attach menu" wide>
          <GalleryOverlayWell>
            <GalleryComposer placeholder="Ask Demi…" attach-open />
          </GalleryOverlayWell>
        </GallerySpecimen>
        <GallerySpecimen variant="queue" wide>
          <GalleryComposer
            placeholder="Ask Demi…"
            running
            draft="Also add a case for the expired cookie."
          />
        </GallerySpecimen>
        <GallerySpecimen variant="stop" wide>
          <GalleryComposer placeholder="Ask Demi…" running />
        </GallerySpecimen>
        <GallerySpecimen variant="fast" wide>
          <GalleryComposer placeholder="Ask Demi…" service-tier-id="priority" />
        </GallerySpecimen>
        <GallerySpecimen variant="no reasoning" wide>
          <GalleryComposer
            placeholder="Ask Demi…"
            selected-provider-id="openai"
            selected-model-id="gpt-5"
          />
        </GallerySpecimen>
        <GallerySpecimen variant="context · warning" wide>
          <GalleryComposer placeholder="Ask Demi…" :usage="usageAt(0.75)" />
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="ModelSelector" note="Chip, and the menu behind it: Fast Mode, Reasoning, and Model. Submenus are on the Overlays page.">
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

    <GallerySection title="SessionDockChip" note="28px capsule with status dots and the agents cluster.">
      <div class="specimen-row">
        <GallerySpecimen variant="resume">
          <SessionDockChip>
            <Play :size="ICON_PX.in28" />
            Resume
          </SessionDockChip>
        </GallerySpecimen>
        <GallerySpecimen variant="running">
          <SessionDockChip dot="accent">3 Running</SessionDockChip>
        </GallerySpecimen>
        <GallerySpecimen variant="ready">
          <SessionDockChip dot="success">Ready</SessionDockChip>
        </GallerySpecimen>
        <GallerySpecimen variant="idle">
          <SessionDockChip dot="muted">Idle</SessionDockChip>
        </GallerySpecimen>
        <GallerySpecimen variant="agents">
          <SessionDockChip>
            <ExploratoryMark kind="cluster" />
            5 Agents
          </SessionDockChip>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="UserBlock" note="User, attachments, overflow, pending steer, queued, and stuck.">
      <div class="specimen-stack specimen-stack-loose">
        <GallerySpecimen variant="user" wide>
          <div class="gallery-frame gallery-user-frame bg-surface">
            <UserBlock :content="userBubble" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="attachments" wide>
          <div class="gallery-frame gallery-user-frame bg-surface">
            <UserBlock :content="attachmentBubble" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="overflow" wide>
          <div class="gallery-frame gallery-user-frame bg-surface">
            <div class="gallery-user-overflow">
              <UserBlock :content="overflowBubble" />
            </div>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="pending steer" wide>
          <div class="gallery-frame gallery-user-frame gallery-user-frame-actions bg-surface">
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
        <GallerySpecimen variant="queued" wide>
          <div class="gallery-frame gallery-user-frame gallery-user-frame-actions bg-surface">
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
        <GallerySpecimen variant="stuck" wide>
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

    <GallerySection title="FunctionalBlock" note="Collapsed, expanded, loading, tool detail, and error.">
      <div class="gallery-frame gallery-block-frame bg-surface">
        <div class="specimen-stack">
          <GallerySpecimen variant="collapsed" wide>
            <FunctionalBlock v-model:open="functionalCollapsed" label="Thought for 8s">
              <template #icon>
                <Brain :size="ICON_PX.in28" />
              </template>
              <template #body>
                <div class="text-conversation text-fg-body">The helper already writes the new session cookie.</div>
              </template>
            </FunctionalBlock>
          </GallerySpecimen>
          <GallerySpecimen variant="expanded" wide>
            <FunctionalBlock v-model:open="functionalExpanded" label="Thought for 8s">
              <template #icon>
                <Brain :size="ICON_PX.in28" />
              </template>
              <template #body>
                <div class="text-conversation text-fg-body">The cookie name changed from sid to session. The test is the one still looking for sid.</div>
              </template>
            </FunctionalBlock>
          </GallerySpecimen>
          <GallerySpecimen variant="loading" wide>
            <FunctionalBlock loading label="Thinking">
              <template #icon>
                <Brain :size="ICON_PX.in28" />
              </template>
            </FunctionalBlock>
          </GallerySpecimen>
          <GallerySpecimen variant="tool · detail" wide>
            <FunctionalBlock
              v-model:open="functionalTool"
              label="shell_exec"
              detail="rg -n &quot;sid&quot; packages/web/src/auth.test.ts"
            >
              <template #icon>
                <Terminal :size="ICON_PX.in28" />
              </template>
              <template #body>
                <pre class="font-mono text-xs text-fg-muted">packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")</pre>
              </template>
            </FunctionalBlock>
          </GallerySpecimen>
          <GallerySpecimen variant="error" wide>
            <FunctionalBlock
              v-model:open="functionalError"
              tone="danger"
              label="Provider aborted after 3 retries."
              error-text="http 429 · rate_limited"
            />
          </GallerySpecimen>
        </div>
      </div>
    </GallerySection>

    <GallerySection title="LoadingBlock" note="Requesting row before the first block.">
      <GallerySpecimen variant="requesting" wide>
        <div class="gallery-frame gallery-activity-frame bg-surface">
          <LoadingBlock />
        </div>
      </GallerySpecimen>
    </GallerySection>

    <GallerySection title="ActivitySlot" note="Connecting, resuming, retrying, and requesting.">
      <GallerySpecimen variant="connecting" wide>
        <div class="gallery-frame gallery-activity-frame bg-surface">
          <ActivitySlot kind="connecting" label="Connecting" />
        </div>
      </GallerySpecimen>
    </GallerySection>

    <GallerySessionPane ref="fullPane" label="Session" tall>
      <GalleryTranscript
        :blocks="sessionBlocks"
        :streaming-thinking-id="sessionStreamingId ?? 'thinking-streaming'"
        :streaming-text-id="sessionTextId"
        :ended-at-by-id="sessionEndedAt"
        :activity="sessionSlot"
        @delete-pending-steer="deletePendingSteer"
        @interrupt-pending-steer="interruptPendingSteer"
        @delete-queued="removeQueued"
        @send-queued="sendNow"
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
            <SessionDockChip dot="accent">3 Running</SessionDockChip>
            <SessionDockChip>
              <ExploratoryMark kind="cluster" />
              5 Agents
            </SessionDockChip>
          </template>
          <GalleryComposer
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
    </GallerySessionPane>

    <GallerySessionPane ref="turnPane" label="Turn">
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
            <SessionDockChip @click="playTurn('retry')">
              Retry
            </SessionDockChip>
            <SessionDockChip @click="playTurn('connect')">
              Connect
            </SessionDockChip>
          </template>
        </SessionDock>
      </template>
    </GallerySessionPane>

    <GallerySection title="Stream" note="Thinking then reply, same reveal.">
      <div class="mb-3">
        <Button variant="ghost" size="sm" @click="playStream">Replay</Button>
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
  </div>
</template>

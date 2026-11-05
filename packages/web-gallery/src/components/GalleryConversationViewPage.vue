<script setup lang="ts">
import { reactive, ref } from 'vue'
import ConversationView from '@demicodes/web-ui/view/ConversationView.vue'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { productWould } from '../product-would'
import { conversationViewAgent, gallerySubagents } from '../fixtures/subagents'
import { galleryTerminals } from '../fixtures/terminals'
import { gallerySchedules } from '../fixtures/schedules'
import { SHAPES, useCardCost } from '../fixtures/cost'

/**
 * A conversation's view link page, over a conversation the gallery holds: its
 * root agent's transcript, the dock's chips and their windows, and the bar
 * under them, as an integration's product frames it.
 */
function conversationOf(root: SubagentRecord, subagents: SubagentRecord[], terminals: TerminalRecord[]): ChatSessionState {
  return reactive({
    id: root.id,
    cwd: root.cwd,
    title: root.name,
    history: root.history,
    shownAt: null,
    summaries: {},
    queue: [],
    pendingSteers: [],
    starting: null,
    pendingCalls: [],
    waitingSince: null,
    requestOutputs: {},
    phase: root.sessionPhase,
    load: 'ready',
    lastError: null,
    pendingAction: null,
    resuming: false,
    failures: {},
    archived: false,
    scroll: null,
    subagents,
    terminals,
    turnSpeed: null,
  })
}

const running = conversationViewAgent('running')
const finished = conversationViewAgent('completed')
// Between turns the children the turn ran have finished too.
const ended = () => gallerySubagents().map((agent) => agent.phase === 'running'
  ? { ...agent, phase: 'completed' as const, sessionPhase: 'idle' as const, endedAt: agent.startedAt }
  : agent)
const services = () => galleryTerminals().filter((terminal) => terminal.service === true)
const live = conversationOf(running, gallerySubagents(), galleryTerminals())
const between = conversationOf(finished, ended(), services())
const narrow = conversationOf(finished, ended(), services())
const schedules = gallerySchedules()
const cost = useCardCost('none', { shape: SHAPES.sixty })
const output: RequestOutputReading = { requestId: 'request-gallery-view', tokens: 860, firstTokenAt: Date.now() - 6_000, endedAt: null }
/** The child each specimen's Agents window shows, which the page follows. */
const followed = ref<Record<string, string | null>>({})
</script>

<template>
  <GallerySection
    title="Conversation View Page"
    note="What a conversation's view link an integration made opens, without an account, in its own window or in a frame of the integration's product: what the conversation's page shows, live, for someone who only watches, with no composer. Its calls, thinking and commands open as the product's do. The dock's chips, Agents, Running, Services and Scheduled, open their windows and menu as on the conversation's page: an agent's link opens the Agents window on its tab, a child's transcript and bar show there, and a command's output in Running. Nothing acts on the conversation: no Stop Agent, Stop All, End Command, Compact, Fork or edit, and a schedule's switch is disabled, with no Delete. A file or a branch is its text, since the page reaches no Host. Along its bottom edge, the bar the Agents window shows along a child's: the state and how long the turn has run, the folder, the profile, the model and its effort, and the context indicator, whose card holds the context and what the conversation cost, without Compact, usage limits or instructions."
  >
    <GallerySpecimen variant="a turn running" wide>
      <div class="gallery-frame h-[38rem]">
        <ConversationView
          :conversation="live"
          :run="running"
          :usage="running.contextUsage"
          :schedules="schedules"
          :cost="cost.cost.value"
          :agent-cost="() => cost.cost.value"
          :output="output"
          @show-subagent="(id) => (followed.live = id)"
          @retry-load="productWould('Load the Conversation Again')"
        />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="between turns" wide>
      <div class="gallery-frame h-[38rem]">
        <ConversationView
          :conversation="between"
          :run="finished"
          :usage="finished.contextUsage"
          :schedules="schedules"
          :cost="cost.cost.value"
          :agent-cost="() => cost.cost.value"
          :output="null"
          @show-subagent="(id) => (followed.between = id)"
          @retry-load="productWould('Load the Conversation Again')"
        />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="a phone-wide frame">
      <div class="gallery-frame h-[38rem] w-[24rem]">
        <ConversationView
          :conversation="narrow"
          :run="finished"
          :usage="finished.contextUsage"
          :schedules="schedules"
          :cost="cost.cost.value"
          :output="null"
          @show-subagent="(id) => (followed.narrow = id)"
          @retry-load="productWould('Load the Conversation Again')"
        />
      </div>
    </GallerySpecimen>
  </GallerySection>
</template>

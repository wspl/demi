<script setup lang="ts">
import { computed, reactive } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import { shortTranscriptBlocks } from '../fixtures/blocks'
import { gallerySubagents } from '../fixtures/subagents'
import { runningRunCommand } from '../fixtures/terminals'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { productWould } from '../product-would'
import GalleryComposer from './GalleryComposer.vue'

/**
 * A conversation the page opens, step by step, as the product's store sets
 * its state: the reads, then the transcript, which waits for nothing else, then
 * the session with its turn and its running command; the agents join whenever
 * their list arrives. The buttons stand in for the backend's answers.
 */
const session = reactive<ChatSessionState>({
  id: 'opening-steps',
  cwd: WORKSPACE_ROOT,
  title: 'Fix the login test',
  history: EMPTY_TRANSCRIPT,
  shownAt: null,
  summaries: {},
  queue: [],
  pendingSteers: [], starting: null,
  pendingCalls: [],
  waitingSince: null, requestOutput: null, turnSpeed: null,
  phase: 'idle',
  load: 'loading',
  lastError: null,
  pendingAction: null,
  failures: {},
  archived: false,
  scroll: null,
  subagents: [],
  terminals: [],
})

/** Back to the moment the user opened it: its reads are on their way. */
function startOver(): void {
  Object.assign(session, {
    history: EMPTY_TRANSCRIPT,
    phase: 'idle',
    load: 'loading',
    subagents: [],
    terminals: [],
  })
}

/** The first step: the transcript and the draft read, the session opening. */
function readHistory(): void {
  Object.assign(session, {
    history: wholeHistory(shortTranscriptBlocks()),
    load: 'opening',
  })
}

/** The agents' list arrives, at either step: what names them joins the conversation. */
function listAgents(): void {
  session.subagents = gallerySubagents().slice(0, 2)
}

/** The second step: the session's handshake, with its turn and its running command. */
function openSession(): void {
  Object.assign(session, {
    phase: 'running',
    terminals: [runningRunCommand()],
    load: 'ready',
  })
}

const step = computed(() => {
  switch (session.load) {
    case 'loading':
      return { label: 'Read History', next: readHistory }
    case 'opening':
    case 'connecting':
      return { label: 'Open Session', next: openSession }
    default:
      return null
  }
})
</script>

<template>
  <div class="space-y-2">
    <div class="flex h-[26rem] min-w-0 flex-col overflow-hidden rounded-xl border border-line bg-surface-base">
      <ChatSession @show-subagents="() => {}"
        :conversation="session"
        has-provider
        @retry="productWould('Resume the Turn')"
        @retry-load="productWould('Load the Conversation Again')"
        @rename="session.title = $event"
        @save-scroll="(_id, value) => (session.scroll = value)"
        @abort-subagents="productWould('Stop the Agents')"
        @abort-subagent="productWould('Stop the Agent')"
        @abort-terminal="productWould('Stop the Command')"
        @open-aside="productWould('Open the Panel')"
        @pin="productWould('Pin the Conversation')"
        @archive="productWould('Archive the Conversation')"
        @delete="productWould('Delete the Conversation')"
        @retry-undelivered="productWould('Send the Messages Again')"
        @edit-submission="productWould('Put the Message Back in the Composer')"
        @delete-submission="productWould('Delete the Message')"
        @recall-queued="productWould('Edit the Queued Message')"
        @send-queued="productWould('Send the Queued Message Now')"
        @recall-steer="productWould('Edit the Steer')"
        @interrupt-pending-steer="productWould('Send the Steer Now')"
        @regenerate="productWould('Regenerate the Answer')"
        @decide-permission="productWould('Decide the Request')"
        @reveal="productWould('Show the Message')"
        @read-subagent="productWould('Read the Agent’s Transcript')"
        @compact-subagent="productWould('Compact the Agent’s Context')"
        @refresh-limits="productWould('Read the Usage Limits Again')"
        @open-instruction="productWould('Open the Instructions')"
      >
        <template #workspace><span class="text-chrome text-fg-muted">Local workspace</span></template>
        <template #composer>
          <GalleryComposer
            :running="session.phase === 'running'"
            @send="productWould('Send the Message')"
            @stop="session.phase = 'idle'"
          />
        </template>
      </ChatSession>
    </div>
    <div class="flex items-center gap-2">
      <Button v-if="step" size="sm" @click="step.next">{{ step.label }}</Button>
      <Button v-if="session.load !== 'loading' && session.subagents.length === 0" size="sm" variant="ghost" @click="listAgents">
        List the Agents
      </Button>
      <Button v-if="session.load === 'opening'" size="sm" variant="ghost" @click="session.load = 'connecting'">
        Lose the Socket
      </Button>
      <Button v-if="session.load !== 'loading'" size="sm" variant="ghost" @click="startOver">Start Over</Button>
    </div>
  </div>
</template>

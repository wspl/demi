<script setup lang="ts">
import { onBeforeUnmount, reactive, ref } from 'vue'
import { delay } from '@demicodes/utils'
import DockWindowArea from '@demicodes/web-ui/agent/DockWindowArea.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { useAgentContext } from '../agent-context'
import { dockWindowShare } from '../dock-window-share'
import { briefSubagent } from '../fixtures/subagents'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * The Agents window over a child whose transcript the page does not hold
 * yet: a live child waits for the transcript its session sends once the
 * page shows it, and a finished one reads its latest page, which may fail
 * and Retry reads again. Each arrival and read here stands in for the
 * product's.
 */
const agentContext = useAgentContext()
const lifetime = new AbortController()
onBeforeUnmount(() => lifetime.abort())

/** A live child the page holds none of. */
function unread(): SubagentRecord {
  return { ...briefSubagent(), history: EMPTY_TRANSCRIPT }
}

/** A finished child the page holds none of. */
function unreadFinished(): SubagentRecord {
  return { ...unread(), phase: 'completed', sessionPhase: 'idle', endedAt: new Date().toISOString() }
}

/** The page the read brings: the child's transcript as the fixture holds it. */
function arrive(agent: SubagentRecord): void {
  agent.history = briefSubagent().history
}

// Loading: a live child waits for its transcript, which Finish Loading
// brings as the session's reset would.
const loadingAgents = reactive([unread()])
const loadingOpen = ref(true)
const loadingId = ref<string | null>(loadingAgents[0]!.id)
const loadingKey = ref(0)
function finishLoading(): void {
  arrive(loadingAgents[0]!)
}
function loadAgain(): void {
  loadingAgents.splice(0, 1, unread())
  loadingOpen.value = true
  loadingId.value = loadingAgents[0]!.id
  loadingKey.value += 1
}

// Failed: a finished child's first read fails; Retry's read brings the page a moment later.
const failedAgents = reactive([unreadFinished()])
const failedOpen = ref(true)
const failedId = ref<string | null>(failedAgents[0]!.id)
const failedKey = ref(0)
let failedReads = 0
async function readFailing(): Promise<void> {
  failedReads += 1
  if (failedReads === 1) {
    throw new Error('The server took too long to answer.')
  }
  await delay(800, lifetime.signal)
  arrive(failedAgents[0]!)
}
function failAgain(): void {
  failedReads = 0
  failedAgents.splice(0, 1, unreadFinished())
  failedOpen.value = true
  failedId.value = failedAgents[0]!.id
  failedKey.value += 1
}

/** Stop Agent or Stop All on a specimen's child: it stops, and its tab stays while the window shows it. */
function stop(agents: SubagentRecord[]): void {
  const agent = agents[0]!
  if (agent.phase === 'running') {
    agent.phase = 'aborted'
    agent.endedAt = new Date().toISOString()
  }
}
</script>

<template>
  <div class="grid grid-cols-1 gap-3 xl:grid-cols-2">
    <GallerySpecimen variant="Loading" wide>
      <div class="mb-3 flex flex-wrap gap-2">
        <Button variant="ghost" size="sm" @click="finishLoading">Finish Loading</Button>
        <Button variant="ghost" size="sm" @click="loadAgain">Load Again</Button>
      </div>
      <DockWindowArea v-model:share="dockWindowShare" class="h-[24rem]">
        <SubagentPanel
          :key="loadingKey"
          v-model:open="loadingOpen"
          v-model:active-id="loadingId"
          :agents="loadingAgents"
          streamed
          @abort="stop(loadingAgents)"
          @abort-agent="stop(loadingAgents)"
          v-bind="agentContext.panel"
        />
      </DockWindowArea>
    </GallerySpecimen>
    <GallerySpecimen variant="Failed" wide>
      <div class="mb-3 flex flex-wrap gap-2">
        <Button variant="ghost" size="sm" @click="failAgain">Fail Again</Button>
      </div>
      <DockWindowArea v-model:share="dockWindowShare" class="h-[24rem]">
        <SubagentPanel
          :key="failedKey"
          v-model:open="failedOpen"
          v-model:active-id="failedId"
          :agents="failedAgents"
          :read-latest="readFailing"
          @abort="stop(failedAgents)"
          @abort-agent="stop(failedAgents)"
          v-bind="agentContext.panel"
        />
      </DockWindowArea>
    </GallerySpecimen>
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { Block, CommandReport, SessionPhase } from '@demicodes/protocol'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { demoModel, shellView } from '../fixtures/blocks'

/**
 * A command that outlives its call, from start to the answer its end wakes
 * (`runtime.md` § Command reports): the call returns while the suite runs
 * and the turn ends; a report of progress wakes the model without a row of
 * its own; the end shows as a notice where it arrived, which starts the
 * next turn, Requesting under it until the answer comes. The notice's title
 * brings the call that started the command into view. Each pause of the
 * real run, 30 seconds between reports, is a moment here.
 */
const STEP_MS = 1_600
const blocks = ref<Block[]>([])
const phase = ref<SessionPhase>('idle')
const stage = ref('')
const timers = new Set<ReturnType<typeof setTimeout>>()

const created = () => new Date().toISOString()
const title = 'Run the end-to-end suite'
const input = JSON.stringify({ script: 'sh scripts/e2e.sh', description: title, intervalMs: 30_000 })
function call(status: 'executing' | 'completed'): Block {
  return {
    type: 'tool_call', id: 'wake-call', createdAt: created(), model: demoModel,
    toolUseId: 'wake-call-use', toolName: 'shell', input, status,
    output: status === 'executing' ? [] : [{ type: 'text', text: 'status: running\ncommandId: 4\noutput:\nscenario 1 ok\nscenario 5 ok\n' }],
    view: status === 'executing' ? null : shellView({ commandId: '4', status: 'running', runningMs: 30_000, chunks: [{ stream: 'stdout', text: 'scenario 1 ok\n…\nscenario 5 ok\n' }] }),
  }
}
function report(id: string, event: CommandReport['event'], output: string): Block {
  return {
    type: 'wakeup', id, turnId: `${id}-turn`, createdAt: created(), model: demoModel,
    placement: 'new_turn', reports: [{ commandId: '4', title, event, output }],
  }
}
function text(id: string, value: string): Block {
  return { type: 'text', id, createdAt: created(), model: demoModel, text: value, forkable: true }
}

/** The run, as the transcript and the session's phase show it, each step a moment after the one before. */
const script: { stage: string; act: () => void }[] = [
  { stage: 'The user asks for the suite.', act: () => {
    blocks.value = [{ type: 'user', id: 'wake-user', turnId: 'wake-turn', createdAt: created(), model: demoModel, content: [{ type: 'text', text: 'Run the end-to-end suite and tell me how it went.' }], preamble: null }]
    phase.value = 'running'
  } },
  { stage: 'The model starts it: the call watches the suite for its interval, 30 seconds.', act: () => {
    blocks.value = [...blocks.value, call('executing')]
  } },
  { stage: 'The suite runs on past the interval: the call returns its handle, and the model ends its turn.', act: () => {
    blocks.value = [...blocks.value.slice(0, -1), call('completed'), text('wake-started', 'The end-to-end suite is running, 30 scenarios in about two and a half minutes. I will tell you when it ends.')]
    phase.value = 'idle'
  } },
  { stage: '30 seconds later, a report of progress wakes the model. It has no row: the answer says what it learned.', act: () => {
    blocks.value = [...blocks.value, report('wake-progress', { kind: 'running', runningMs: 60_000, idleMs: 2_000, intervalMs: 30_000 }, 'scenario 6 ok\n…\nscenario 10 ok')]
    phase.value = 'running'
  } },
  { stage: 'The model answers the progress and ends its turn again.', act: () => {
    blocks.value = [...blocks.value, text('wake-progress-answer', 'Still running: scenarios 6 to 10 passed, none failed.')]
    phase.value = 'idle'
  } },
  { stage: 'The suite ends: its notice shows where the report arrived and starts a turn, Requesting under it.', act: () => {
    blocks.value = [...blocks.value, report('wake-end', { kind: 'ended', exitCode: 0 }, 'scenario 30 ok\n30 scenarios passed')]
    phase.value = 'running'
  } },
  { stage: 'The model answers from the report’s output, without looking at the command again. Click the notice’s title to see the call it names.', act: () => {
    blocks.value = [...blocks.value, text('wake-answer', 'All 30 end-to-end scenarios passed in 2 minutes 31 seconds.')]
    phase.value = 'idle'
  } },
]

function play(): void {
  for (const timer of timers)
    clearTimeout(timer)
  timers.clear()
  script.forEach((step, index) => {
    const timer = setTimeout(() => {
      timers.delete(timer)
      stage.value = step.stage
      step.act()
    }, index * STEP_MS)
    timers.add(timer)
  })
}
onMounted(play)
onBeforeUnmount(() => {
  for (const timer of timers)
    clearTimeout(timer)
})
</script>

<template>
  <GallerySection
    title="A Command’s End Wakes the Agent"
    note="A command that outlives its call reports to the model as it runs and when it ends; the model never asks. Reports of progress show nothing, since the answer they wake says what the model learned. The end shows as a notice where it arrived, with a bell, as the cause of the turn it starts: the call's title is a link to the call that started the command. A failed, stopped or lost command says so, with the tag a shell row carries."
  >
    <GallerySpecimen variant="the whole run, its pauses shortened" wide>
      <div class="flex flex-col gap-2">
        <div class="gallery-frame h-[26rem] bg-surface">
          <AgentMessageList
            class="h-full"
            conversation-id="gallery-command-wakeup"
            :blocks="blocks"
            :pending-steers="[]"
            :queue="[]"
            :phase="phase"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
          />
        </div>
        <div class="flex items-center gap-3">
          <Button size="sm" @click="play">Replay</Button>
          <span class="text-[13px] text-fg-muted">{{ stage }}</span>
        </div>
      </div>
    </GallerySpecimen>
  </GallerySection>
</template>

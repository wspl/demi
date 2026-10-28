<script setup lang="ts">
import { ref } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { demoModel, transcriptDemoBlocks } from '../fixtures/blocks'
import { PROJECT_ROOT } from '../fixtures/project'
import { productWould } from '../product-would'
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * A conversation the page opens again from its copy while its session went
 * on: the spinner over the transcript's end says the page is behind until
 * what the session did since joins it. Switch Back reads it again; Lose the
 * Connection drops its lane for four seconds.
 */
const flow = useTurnFlow({ id: 'gallery-catching-up', title: 'Login test', cwd: PROJECT_ROOT, blocks: transcriptDemoBlocks() })
const session = flow.state
const catchingUp = ref(false)
/** How long the read again takes: past the spinner's half-second wait. */
const READ_MS = 2500
const LOST_MS = 4000
/** What the session wrote while the page was away, one for each Switch Back. */
const MEANWHILE = [
  'While you were away I reran the login tests: all 42 pass.',
  'I also checked the logout flow; it clears the new cookie too.',
  'The CI run for the branch is green.',
]
let timer: ReturnType<typeof setTimeout> | undefined
let read = 0

/**
 * Shows the conversation again from its copy, as the page does after it let
 * it go: what the session wrote since joins once the read ends, after what
 * the transcript holds by then, a message sent meanwhile included.
 */
function switchBack(): void {
  clearTimeout(timer)
  catchingUp.value = true
  timer = setTimeout(() => {
    read += 1
    session.blocks = [
      ...session.blocks,
      {
        type: 'text',
        id: `catch-up-${read}`,
        createdAt: new Date().toISOString(),
        model: demoModel,
        text: MEANWHILE[(read - 1) % MEANWHILE.length]!,
      },
    ]
    catchingUp.value = false
  }, READ_MS)
}

function loseConnection(): void {
  session.load = 'reconnecting'
  setTimeout(() => {
    session.load = 'ready'
  }, LOST_MS)
}

switchBack()
</script>

<template>
  <div class="flex gap-2 pb-3">
    <Button size="sm" @click="switchBack">Switch Back</Button>
    <Button size="sm" @click="loseConnection">Lose the Connection</Button>
  </div>
  <div class="gallery-frame flex h-[32rem] flex-col overflow-hidden bg-surface">
    <ChatSession
      @show-subagent="() => {}"
      :conversation="session"
      :catching-up="catchingUp"
      has-provider
      :pending-submissions="flow.pendingSubmissions.value"
      :removing="flow.removing.value"
      @rename="session.title = $event"
      @save-scroll="(_id, state) => (session.scroll = state)"
      @retry="flow.resume()"
      @retry-undelivered="(id) => flow.retrySubmission(id)"
      @edit-submission="flow.editSubmission"
      @delete-submission="flow.deleteSubmission"
      @recall-queued="flow.recallQueued"
      @send-queued="flow.sendQueued"
      @recall-steer="flow.recallSteer"
      @interrupt-pending-steer="flow.interruptPendingSteer"
      @abort-subagents="flow.abortSubagents"
      @abort-subagent="flow.abortSubagent"
      @abort-terminal="flow.abortTerminal"
      @regenerate="productWould('Regenerate the Answer')"
      @switch-version="productWould('Show the Other Version')"
    >
      <template #composer>
        <GalleryComposer
          :running="session.phase === 'running' || flow.startingTurn.value"
          :flow="flow"
          @send="flow.turn"
          @steer="flow.steer"
          @queue="flow.queue"
          @stop="flow.stop"
          @compact="flow.compact"
        />
      </template>
    </ChatSession>
  </div>
</template>

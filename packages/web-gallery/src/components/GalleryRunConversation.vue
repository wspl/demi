<script setup lang="ts">
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { provideAutomationOpener } from '@demicodes/web-ui/agent/message-origins'
import AutomationRunBar from '@demicodes/web-ui/automations/AutomationRunBar.vue'
import type { Automation, AutomationRun } from '@demicodes/web-ui/automations/types'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { runConversationBlocks } from '../fixtures/automations'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * A run's conversation (`automations.md` § Runs): the bar under its header
 * while it belongs to its automation, and the automation's message, marked
 * with the automation. A message the user sends moves it to Conversations,
 * as Move to Conversations does.
 */
const props = defineProps<{
  automation: Automation
  run: AutomationRun
  title: string
}>()

const emit = defineEmits<{
  openPage: []
  openAutomation: []
  move: []
}>()

const flow = useTurnFlow({
  id: props.run.conversationId ?? props.run.id,
  title: props.title,
  cwd: WORKSPACE_ROOT,
  blocks: runConversationBlocks(props.automation, props.run),
})
const session = flow.state
const messageFiles = useMessageFiles()

// The automation's message leads back to it.
provideAutomationOpener(() => emit('openAutomation'))

function send(...args: Parameters<typeof flow.turn>): void {
  flow.turn(...args)
  if (props.run.filed) {
    emit('move')
  }
}
</script>

<template>
  <ChatSession @show-subagents="() => {}"
    :conversation="session"
    :files="messageFiles()"
    has-provider
    @rename="session.title = $event"
    @save-scroll="(_id, state) => (session.scroll = state)"
    @retry="flow.resume()"
  >
    <template v-if="run.filed" #bar>
      <AutomationRunBar
        :name="automation.name"
        :at="run.at"
        :time-zone="automation.timeZone"
        @open-page="emit('openPage')"
        @open-automation="emit('openAutomation')"
        @move="emit('move')"
      />
    </template>
    <template #composer>
      <GalleryComposer
        :running="session.phase === 'running' || flow.startingTurn.value"
        :flow="flow"
        @send="send"
        @steer="flow.steer"
        @queue="flow.queue"
        @stop="flow.stop"
        @compact="flow.compact"
      />
    </template>
  </ChatSession>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { useAgentWorkspace } from './workspace'
import AgentMessageList from './AgentMessageList.vue'
import AgentMessageInput from './AgentMessageInput.vue'
import MessageQueueBar from './MessageQueueBar.vue'
import { queuedMessageIdForEmptySubmit } from './queue-submit'
import { reportError } from '@demicodes/web-ui/infra/errors'

const props = defineProps<{
  conversationId: string
}>()

const workspace = useAgentWorkspace()
const session = computed(() => workspace.sessions[props.conversationId])
const blocks = computed(() => session.value?.blocks ?? [])
const queuedMessages = computed(() => session.value?.queue ?? [])
const pendingSteers = computed(() => session.value?.pendingSteers ?? [])
const phase = computed(() => session.value?.phase ?? 'idle')

const bottomAreaRef = ref<HTMLDivElement>()
const { height: bottomAreaHeight } = useElementSize(bottomAreaRef)

function handleEmptySubmit() {
  const messageId = queuedMessageIdForEmptySubmit(queuedMessages.value)
  if (!messageId) return
  handleQueuedSendNow(messageId)
}

function handleQueuedSendNow(messageId: string) {
  if (phase.value === 'running') {
    void workspace.steerQueuedMessage(props.conversationId, messageId).catch((error) => {
      reportError('Failed to steer queued message', error, { userVisible: true })
    })
    return
  }
  workspace.sendQueuedMessage(props.conversationId, messageId)
}

function onContinue() {
  void workspace.resume(props.conversationId)
}

function onRetry() {
  void workspace.retry(props.conversationId)
}
</script>

<template>
  <div class="relative h-full flex-1 overflow-hidden bg-surface">
    <AgentMessageList
      :conversation-id="conversationId"
      :blocks="blocks"
      :pending-steers="pendingSteers"
      :phase="phase"
      :bottom-offset="bottomAreaHeight"
      :persisted-scroll-state="undefined"
      @continue="onContinue"
      @retry="onRetry"
      @delete-pending-steer="(steerId) => workspace.deletePendingSteer(conversationId, steerId)"
    />
    <div class="absolute bottom-0 left-0 right-0 z-10 px-3 pb-3">
      <div ref="bottomAreaRef" class="relative">
        <MessageQueueBar
          v-if="queuedMessages.length"
          :messages="queuedMessages"
          @remove="(messageId) => workspace.dequeueMessage(conversationId, messageId)"
          @send-now="handleQueuedSendNow"
          @clear-all="workspace.clearMessageQueue(conversationId)"
        />
        <AgentMessageInput
          class="relative z-10"
          :conversation-id="conversationId"
          @empty-submit="handleEmptySubmit"
        />
      </div>
    </div>
  </div>
</template>

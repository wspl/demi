<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { useAgentWorkspace } from './workspace'
import AgentMessageList from './AgentMessageList.vue'
import AgentMessageInput from './AgentMessageInput.vue'
import MessageQueueBar from './MessageQueueBar.vue'
import { queuedMessageIdForEmptySubmit } from './queue-submit'

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
const messageInputRef = ref<InstanceType<typeof AgentMessageInput>>()
const { height: bottomAreaHeight } = useElementSize(bottomAreaRef)

function handleEmptySubmit() {
  const messageId = queuedMessageIdForEmptySubmit(queuedMessages.value)
  if (!messageId) return
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
    <div class="absolute bottom-0 left-0 right-0 z-10 px-5 pb-4">
      <div ref="bottomAreaRef" class="relative">
        <MessageQueueBar
          v-if="queuedMessages.length"
          :messages="queuedMessages"
          @remove="(messageId) => workspace.dequeueMessage(conversationId, messageId)"
          @send-now="(messageId) => workspace.sendQueuedMessage(conversationId, messageId)"
          @clear-all="workspace.clearMessageQueue(conversationId)"
        />
        <AgentMessageInput
          ref="messageInputRef"
          class="relative z-10"
          :conversation-id="conversationId"
          @empty-submit="handleEmptySubmit"
        />
      </div>
    </div>
  </div>
</template>

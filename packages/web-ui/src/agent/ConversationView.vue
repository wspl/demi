<script setup lang="ts">
import { computed, ref } from 'vue'
import { Play } from '@lucide/vue'
import { useAgentWorkspace } from './workspace'
import AgentMessageList from './AgentMessageList.vue'
import AgentMessageInput from './AgentMessageInput.vue'
import SessionDock from './SessionDock.vue'
import SessionDockChip from './SessionDockChip.vue'
import SessionSurface from './SessionSurface.vue'
import { canResumeFromDock } from './dock-recovery'
import { queuedMessageIdForEmptySubmit } from './queue-submit'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { t } from '@demicodes/web-ui/infra/i18n'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

const props = defineProps<{
  conversationId: string
}>()

const workspace = useAgentWorkspace()
const session = computed(() => workspace.sessions[props.conversationId])
const blocks = computed(() => session.value?.blocks ?? [])
const queuedMessages = computed(() => session.value?.queue ?? [])
const pendingSteers = computed(() => session.value?.pendingSteers ?? [])
const phase = computed(() => session.value?.phase ?? 'idle')
const canResume = computed(() => canResumeFromDock(phase.value, blocks.value))

const listRef = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()
const surfaceRef = ref<{ dockHeight: number }>()
const showScrollToBottom = computed(() => Boolean(listRef.value && !listRef.value.isAtBottom))

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

function handleInterruptPendingSteer(steerId: string) {
  void workspace.interruptPendingSteer(props.conversationId, steerId).catch((error) => {
    reportError('Failed to interrupt the current turn', error, { userVisible: true })
  })
}

function handleResume() {
  void workspace.resume(props.conversationId).catch((error) => {
    reportError('Failed to resume conversation', error, { userVisible: true })
  })
}
</script>

<template>
  <SessionSurface ref="surfaceRef">
    <AgentMessageList
      ref="listRef"
      :conversation-id="conversationId"
      :blocks="blocks"
      :pending-steers="pendingSteers"
      :queue="queuedMessages"
      :phase="phase"
      :bottom-offset="surfaceRef?.dockHeight ?? 0"
      :persisted-scroll-state="undefined"
      @delete-pending-steer="(steerId) => workspace.deletePendingSteer(conversationId, steerId)"
      @interrupt-pending-steer="handleInterruptPendingSteer"
      @delete-queued="(messageId) => workspace.dequeueMessage(conversationId, messageId)"
      @send-queued="handleQueuedSendNow"
    />
    <template #dock>
      <SessionDock
        :show-scroll-to-bottom="showScrollToBottom"
        @scroll-to-bottom="listRef?.scrollToBottom()"
      >
        <template #chips>
          <SessionDockChip v-if="canResume" @click="handleResume">
            <Play :size="ICON_PX.in28" />
            {{ t('agent.dock.resume') }}
          </SessionDockChip>
        </template>
        <AgentMessageInput
          :conversation-id="conversationId"
          @empty-submit="handleEmptySubmit"
        />
      </SessionDock>
    </template>
  </SessionSurface>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { useAgentWorkspace } from './workspace'
import AgentMessageList from './AgentMessageList.vue'
import AgentMessageInput from './AgentMessageInput.vue'

const props = defineProps<{
  conversationId: string
}>()

const workspace = useAgentWorkspace()
const session = computed(() => workspace.sessions[props.conversationId])
const blocks = computed(() => session.value?.blocks ?? [])
const phase = computed(() => session.value?.phase ?? 'idle')

const bottomAreaRef = ref<HTMLDivElement>()
const { height: bottomAreaHeight } = useElementSize(bottomAreaRef)

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
      :phase="phase"
      :bottom-offset="bottomAreaHeight"
      :persisted-scroll-state="undefined"
      @continue="onContinue"
      @retry="onRetry"
    />
    <div class="absolute bottom-0 left-0 right-0 z-10 px-5 pb-4">
      <div ref="bottomAreaRef" class="relative">
        <AgentMessageInput :conversation-id="conversationId" />
      </div>
    </div>
  </div>
</template>

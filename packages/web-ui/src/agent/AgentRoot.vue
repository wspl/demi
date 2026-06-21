<script setup lang="ts">
import { computed } from 'vue'
import { AgentWorkspace, provideAgentWorkspace } from './workspace'
import ConversationView from './ConversationView.vue'

const props = defineProps<{
  workspace: AgentWorkspace
}>()

provideAgentWorkspace(props.workspace)

const activeId = computed(() => props.workspace.activeId.value)
</script>

<template>
  <div class="flex h-full flex-col bg-surface">
    <ConversationView v-if="activeId" :key="activeId" :conversation-id="activeId" />
    <div v-else class="grid h-full place-items-center text-sm text-fg-faint">No conversation</div>
  </div>
</template>

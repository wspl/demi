<script setup lang="ts">
import { AgentWorkspace, provideAgentWorkspace } from './workspace'
import { provideAgentUiOptions, type AgentUiOptions } from './ui-options'
import AgentPanel from './AgentPanel.vue'
import ToastHost from '../ui/ToastHost.vue'

const props = defineProps<{
  workspace: AgentWorkspace
  uiOptions?: Partial<AgentUiOptions>
}>()

provideAgentWorkspace(props.workspace)
provideAgentUiOptions(props.uiOptions)
</script>

<template>
  <AgentPanel />
  <ToastHost />
</template>

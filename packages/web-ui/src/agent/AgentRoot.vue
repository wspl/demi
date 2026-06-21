<script setup lang="ts">
import { AgentWorkspace, provideAgentWorkspace } from './workspace'
import AgentPanel from './AgentPanel.vue'

const props = defineProps<{
  workspace: AgentWorkspace
}>()

provideAgentWorkspace(props.workspace)
</script>

<template>
  <AgentPanel />
</template>

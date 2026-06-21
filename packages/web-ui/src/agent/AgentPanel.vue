<script setup lang="ts">
import { computed } from 'vue'
import { useAgentWorkspace } from './workspace'
import AgentTabBar from './AgentTabBar.vue'
import ConversationView from './ConversationView.vue'

const workspace = useAgentWorkspace()

const tabs = computed(() => workspace.tabs.value)
const activeTabId = computed(() => workspace.activeId.value)
</script>

<template>
  <div class="flex h-full flex-col">
    <AgentTabBar :tabs="tabs" :all-conversations="tabs" :active-tab-id="activeTabId" />
    <ConversationView v-if="activeTabId" :key="activeTabId" :conversation-id="activeTabId" />
  </div>
</template>

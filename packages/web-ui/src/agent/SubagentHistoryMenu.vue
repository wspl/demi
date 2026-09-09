<script setup lang="ts">
import { computed } from 'vue'
import { Bot, History } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import {
  finishedSubagents,
  formatSubagentDuration,
  type SubagentRecord,
} from './subagents'

const props = defineProps<{
  agents: readonly SubagentRecord[]
  activeId: string | null
}>()

const emit = defineEmits<{
  select: [id: string]
}>()

const items = computed(() => {
  const nowMs = Date.now()
  return finishedSubagents(props.agents).map((agent) => ({
    id: agent.id,
    label: agent.name,
    icon: Bot,
    value: formatSubagentDuration(agent, nowMs),
  }))
})
function select(id: string, close: () => void): void {
  emit('select', id)
  close()
}
</script>

<template>
  <Dropdown
    :overlay-store="appOverlayStore"
    placement="bottom-end"
    :offset="6"
  >
    <template #trigger="{ isOpen }">
      <IconButton
        :icon="History"
        size="sm"
        variant="ghost"
        :pressed="isOpen"
        aria-label="Completed"
      />
    </template>
    <template #content="{ close }">
      <Menu
        class="w-80"
        filterable
        filter-placeholder="Search completed"
        empty-text="No completed sub-agents"
        :items="items"
        :selected-id="activeId ?? undefined"
        :item-height="28"
        @select="select($event, close)"
      />
    </template>
  </Dropdown>
</template>

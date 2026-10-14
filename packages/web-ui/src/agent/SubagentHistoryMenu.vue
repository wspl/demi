<script setup lang="ts">
import { computed } from 'vue'
import { Bot, History } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import HighlightText from '../ui/HighlightText.vue'
import Menu from '../ui/Menu.vue'
import Tag from '../ui/Tag.vue'
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
    profile: agent.profile,
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
        empty-text="No completed subagents"
        :items="items"
        :selected-id="activeId ?? undefined"
        :item-height="28"
        @select="select($event, close)"
      >
        <!-- The child's profile after its description, as a tag; none for no profile (`subagents.md` § What an agent runs on). -->
        <template #item="{ item, query }">
          <span class="flex min-w-0 flex-1 items-center gap-1.5">
            <span class="min-w-0 truncate"><HighlightText :text="item.label" :query="query" /></span>
            <Tag v-if="item.profile" class="shrink-0">{{ item.profile }}</Tag>
          </span>
        </template>
      </Menu>
    </template>
  </Dropdown>
</template>

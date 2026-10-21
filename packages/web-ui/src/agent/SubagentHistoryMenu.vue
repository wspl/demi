<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { Bot, History } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import HighlightText from '../ui/HighlightText.vue'
import Menu from '../ui/Menu.vue'
import Tag from '../ui/Tag.vue'
import {
  agentName,
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
    label: agentName(agent.name, agent.number),
    icon: Bot,
    value: formatSubagentDuration(agent, nowMs),
    profile: agent.profile,
    phase: agent.phase,
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
        :aria-label="t('subagents.completed')"
      />
    </template>
    <template #content="{ close }">
      <Menu
        class="w-80"
        filterable
        :filter-placeholder="t('subagents.searchCompleted')"
        :empty-text="t('subagents.noneCompleted')"
        :items="items"
        :selected-id="activeId ?? undefined"
        :item-height="28"
        @select="select($event, close)"
      >
        <!--
          The child's profile after its description, as a tag; none for no
          profile. A child that failed or was stopped says so after it; one
          that finished says nothing, as every row here ended.
        -->
        <template #item="{ item, query }">
          <span class="flex min-w-0 flex-1 items-center gap-1.5">
            <span class="min-w-0 truncate"><HighlightText :text="item.label" :query="query" /></span>
            <Tag v-if="item.profile" class="shrink-0">{{ item.profile }}</Tag>
            <Tag v-if="item.phase === 'error'" tone="danger" class="shrink-0">{{ t('subagents.phase.error') }}</Tag>
            <Tag v-else-if="item.phase === 'aborted'" class="shrink-0">{{ t('subagents.phase.aborted') }}</Tag>
          </span>
        </template>
      </Menu>
    </template>
  </Dropdown>
</template>

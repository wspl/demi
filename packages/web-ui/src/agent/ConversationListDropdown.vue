<script setup lang="ts">
import { computed } from 'vue'
import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import { History, MessageSquare } from '@lucide/vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'

dayjs.extend(relativeTime)
import { t } from '@demicodes/web-ui/infra/i18n'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import HighlightText from '@demicodes/web-ui/ui/HighlightText.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'

// Open tabs and server-side history summaries both satisfy this shape.
interface ConversationListItem {
  id: string
  title: string
  createdAt: string
}

const props = defineProps<{
  conversations: ConversationListItem[]
  activeTabId: string | null
}>()

const emit = defineEmits<{
  select: [conversationId: string]
}>()

const items = computed(() =>
  props.conversations.map((c) => ({
    id: c.id,
    label: c.title,
    createdAt: c.createdAt,
    icon: MessageSquare,
  })),
)

function formatTime(iso: string): string {
  return dayjs(iso).fromNow()
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
      />
    </template>
    <template #content="{ close }">
      <Menu
        class="w-96"
        filterable
        :filter-placeholder="t('agent.conversationList.placeholder')"
        :empty-text="t('agent.conversationList.empty')"
        :items="items"
        :selected-id="activeTabId ?? undefined"
        :item-height="28"
        @select="emit('select', $event); close()"
      >
        <template #item="{ item, query }">
          <span class="flex min-w-0 flex-1 items-baseline gap-1.5">
            <span class="truncate">
              <HighlightText :text="item.label" :query="query" />
            </span>
            <span class="shrink-0 text-[11px] text-fg-faint">{{ formatTime(item.createdAt) }}</span>
          </span>
        </template>
      </Menu>
    </template>
  </Dropdown>
</template>

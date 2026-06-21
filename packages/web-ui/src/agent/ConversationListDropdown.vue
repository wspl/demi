<script setup lang="ts">
import { computed } from 'vue'
import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import { HistoryLine } from '@mingcute/vue/history'
import { Chat1Line } from '@mingcute/vue/chat-1'
import type { ConversationState } from './types'

dayjs.extend(relativeTime)
import { t } from '@demi/web-ui/infra/i18n'
import { appOverlayStore } from '@demi/web-ui/overlay/appOverlay'
import HighlightText from '@demi/web-ui/ui/HighlightText.vue'
import SelectDropdown from '@demi/web-ui/ui/SelectDropdown.vue'

const props = defineProps<{
  conversations: ConversationState[]
  activeTabId: string | null
}>()

const emit = defineEmits<{
  select: [conversationId: string]
}>()

const items = computed(() =>
  props.conversations.map(c => ({ ...c, label: c.title })),
)

function formatTime(iso: string): string {
  return dayjs(iso).fromNow()
}
</script>

<template>
  <SelectDropdown
    :overlay-store="appOverlayStore"
    :items="items"
    :selected-id="activeTabId ?? undefined"
    searchable
    :search-placeholder="t('agent.conversationList.placeholder')"
    :empty-text="t('agent.conversationList.empty')"
    panel-class="w-96"
    :item-height="32"
    placement="bottom-end"
    :offset="6"
    @select="emit('select', $event)"
  >
    <template #trigger="{ isOpen }">
      <span
        class="flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-fg-faint transition-colors hover:bg-hover hover:text-fg-muted"
        :class="isOpen && 'bg-overlay/6 text-fg-muted'"
      >
        <HistoryLine :size="14" />
      </span>
    </template>
    <template #item="{ item, query }">
      <Chat1Line :size="13" class="shrink-0" />
      <span class="flex min-w-0 flex-1 items-baseline gap-1.5">
        <span class="truncate text-[13px]">
          <HighlightText :text="item.label" :query="query" />
        </span>
        <span class="shrink-0 text-[11px] text-fg-faint">{{ formatTime(item.createdAt) }}</span>
      </span>
    </template>
  </SelectDropdown>
</template>

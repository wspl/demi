<script setup lang="ts">
import AsyncRegion from '../ui/AsyncRegion.vue'
import { computed, ref } from 'vue'
import { Search } from '@lucide/vue'
import Button from '../ui/Button.vue'
import TextInput from '../ui/TextInput.vue'
import { ICON_PX } from '../ui/icon-metrics'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsArchivedConversation } from './types'

/** What was put away: a searchable list, each row with Restore. The host brings one back and opens it. */
const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  pendingIds?: string[]
  conversations: SettingsArchivedConversation[]
}>()

const emit = defineEmits<{
  retry: []
  restore: [id: string]
}>()

const query = ref('')
const shown = computed(() => {
  const q = query.value.trim().toLowerCase()
  return q
    ? props.conversations.filter((conversation) =>
        conversation.title.toLowerCase().includes(q),
      )
    : props.conversations
})
</script>

<template>
  <SettingsPage
    title="Archived"
    description="Conversations put away from the sidebar. Restore brings one back and opens it."
  >
    <SettingsGroup>
      <template #header>
        <TextInput
          v-model="query"
          placeholder="Search archived"
          aria-label="Search archived conversations"
        >
          <template #prefix><Search :size="ICON_PX.in24" /></template>
        </TextInput>
      </template>
      <AsyncRegion
        :state="load"
        label="Loading archived conversations…"
        @retry="emit('retry')"
      >
        <SettingsRow
          v-for="conversation in shown"
          :key="conversation.id"
          :label="conversation.title"
          :description="conversation.detail"
        >
          <Button
            size="sm"
            :loading="pendingIds?.includes(conversation.id)"
            @click="emit('restore', conversation.id)"
            >Restore</Button
          >
        </SettingsRow>
        <div
          v-if="!shown.length"
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          {{
            query ? 'No archived conversation matches.' : 'Nothing is archived.'
          }}
        </div>
      </AsyncRegion>
    </SettingsGroup>
  </SettingsPage>
</template>

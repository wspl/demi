<script setup lang="ts">
import AsyncRegion from '../ui/AsyncRegion.vue'
import { computed, ref } from 'vue'
import { Search } from '@lucide/vue'
import Button from '../ui/Button.vue'
import TextInput from '../ui/TextInput.vue'
import ConversationDeleteDialog from '../sidebar/ConversationDeleteDialog.vue'
import { conversationTitle } from '../agent/conversation-title'
import type { OverlayStore } from '../overlay/overlayStore'
import { ICON_PX } from '../ui/icon-metrics'
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsArchivedConversation } from './types'

/**
 * What was put away: a searchable list. A row opens its conversation to read,
 * with the bar that offers Restore (`conversations.md` § Conversations and
 * projects); its Restore button brings it back and opens it, and its
 * Delete… button asks before the conversation goes for good.
 */
const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  conversations: SettingsArchivedConversation[]
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  retry: []
  /** Open the conversation read-only. */
  open: [id: string]
  restore: [id: string]
  /** Delete, once its dialog was answered. */
  delete: [id: string]
}>()

// The dialog keeps the conversation it asks about while it closes.
const deleting = ref<SettingsArchivedConversation | null>(null)
const deleteOpen = ref(false)

function askDelete(conversation: SettingsArchivedConversation): void {
  deleting.value = conversation
  deleteOpen.value = true
}

function confirmDelete(): void {
  deleteOpen.value = false
  if (deleting.value) {
    emit('delete', deleting.value.id)
  }
}

const query = ref('')
const shown = computed(() => {
  const q = query.value.trim().toLowerCase()
  return q
    ? props.conversations.filter((conversation) =>
        conversationTitle(conversation.title).toLowerCase().includes(q),
      )
    : props.conversations
})
</script>

<template>
  <SettingsPage
    :title="t('archived.title')"
    :description="t('archived.description')"
  >
    <SettingsGroup>
      <template #header>
        <TextInput
          v-model="query"
          search
          :placeholder="t('archived.search')"
          :aria-label="t('archived.searchLabel')"
        >
          <template #prefix><Search :size="ICON_PX.in24" /></template>
        </TextInput>
      </template>
      <AsyncRegion
        :state="load"
        :label="t('archived.loading')"
        @retry="emit('retry')"
      >
        <SettingsRow
          v-for="conversation in shown"
          :key="conversation.id"
          :label="conversationTitle(conversation.title)"
          :description="conversation.detail"
          interactive
          isolate-controls
          @click="emit('open', conversation.id)"
        >
          <Button size="sm" variant="ghost" @click="emit('open', conversation.id)">{{ t('archived.open') }}</Button>
          <Button size="sm" @click="emit('restore', conversation.id)">{{ t('archived.restore') }}</Button>
          <Button size="sm" variant="danger" @click="askDelete(conversation)">{{ t('archived.delete') }}</Button>
        </SettingsRow>
        <div
          v-if="!shown.length"
          class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
        >
          {{ query ? t('archived.noResults') : t('archived.none') }}
        </div>
      </AsyncRegion>
    </SettingsGroup>
    <ConversationDeleteDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :titles="deleting ? [conversationTitle(deleting.title)] : []"
      @close="deleteOpen = false"
      @delete="confirmDelete"
    />
  </SettingsPage>
</template>

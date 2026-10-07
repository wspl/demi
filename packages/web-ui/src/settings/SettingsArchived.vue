<script setup lang="ts">
import AsyncRegion from '../ui/AsyncRegion.vue'
import { computed, ref } from 'vue'
import { Search } from '@lucide/vue'
import Button from '../ui/Button.vue'
import TextInput from '../ui/TextInput.vue'
import ConversationDeleteDialog from '../sidebar/ConversationDeleteDialog.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { ICON_PX } from '../ui/icon-metrics'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsArchivedConversation } from './types'

/**
 * What was put away: a searchable list. A row opens its conversation to read,
 * with the bar that offers Restore (`product.md` § Conversations and
 * projects); its Restore button brings it back and opens it, and its
 * Delete… button asks before the conversation goes for good.
 */
const props = defineProps<{
  load?: 'loading' | 'ready' | 'failed'
  pendingIds?: string[]
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
        conversation.title.toLowerCase().includes(q),
      )
    : props.conversations
})
</script>

<template>
  <SettingsPage
    title="Archived"
    description="Conversations put away from the sidebar. Click one to read it; Restore brings it back, and Delete removes it for good."
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
          interactive
          isolate-controls
          @click="emit('open', conversation.id)"
        >
          <Button size="sm" variant="ghost" @click="emit('open', conversation.id)">Open</Button>
          <Button
            size="sm"
            :loading="pendingIds?.includes(conversation.id)"
            @click="emit('restore', conversation.id)"
            >Restore</Button
          >
          <Button
            size="sm"
            variant="danger"
            :disabled="pendingIds?.includes(conversation.id)"
            @click="askDelete(conversation)"
            >Delete…</Button
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
    <ConversationDeleteDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :titles="deleting ? [deleting.title] : []"
      @close="deleteOpen = false"
      @delete="confirmDelete"
    />
  </SettingsPage>
</template>

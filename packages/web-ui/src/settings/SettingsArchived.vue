<script setup lang="ts">
import { computed, ref } from 'vue'
import { Folder, Search } from '@lucide/vue'
import { conversationTitle } from '../agent/conversation-title'
import type { OverlayStore } from '../overlay/overlayStore'
import ConversationDeleteDialog from '../sidebar/ConversationDeleteDialog.vue'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '../ui/Button.vue'
import Checkbox from '../ui/Checkbox.vue'
import { formatModified } from '../ui/dates'
import { ICON_PX } from '../ui/icon-metrics'
import { usePhone } from '../ui/phone'
import TextInput from '../ui/TextInput.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { t as uiText } from '../locales/text'
import { useListSelection } from './list-selection'
import { t } from './locales/text'
import SettingsPage from './SettingsPage.vue'
import type { SettingsArchivedConversation } from './types'

/**
 * What was put away: a searchable list, one line per conversation, the last
 * archived first.
 *
 * ```
 * 3 Conversations
 * ☐  Search archived
 * ☐  Draft the onboarding email   ▭ api   Sep 2, 2026, 6:12 PM
 * ☐  Compare vector databases          [Open] [Restore]   ← under the pointer
 * ```
 *
 * - A row's end shows its project, if it has one, and when it was last
 *   active, to the minute, which Open and Restore take under the pointer or
 *   the focus, in the same room. Open shows the conversation to read, with
 *   the bar that offers Restore; Restore brings it back and opens it.
 * - Checking rows turns the title's line into the count and Restore and
 *   Delete… for all of them, at the same height; the search row's checkbox
 *   takes every row the search shows.
 * - On a phone, which has no hover, a row opens its conversation, and Select
 *   shows the checkboxes. The project and the time go under the title, on a
 *   second line, so the title keeps the row's width.
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
  /** A row's Restore: restores the conversation and opens it. */
  restore: [id: string]
  /** Restore for the checked rows: restores them, opening none. */
  restoreChecked: [ids: string[]]
  /** Delete, once its dialog was answered. */
  delete: [ids: string[]]
}>()

const phone = usePhone()

const query = ref('')
const shown = computed(() => {
  const q = query.value.trim().toLowerCase()
  return q
    ? props.conversations.filter((conversation) =>
        conversationTitle(conversation.title).toLowerCase().includes(q),
      )
    : props.conversations
})

const ids = computed(() => shown.value.map((conversation) => conversation.id))
const selection = useListSelection(ids)
const checked = computed(() => shown.value.filter((conversation) => selection.selected.value.has(conversation.id)))
const allChecked = computed(() => shown.value.length > 0 && checked.value.length === shown.value.length)
const someChecked = computed(() => checked.value.length > 0 && !allChecked.value)
/** Whether rows show their checkboxes: always on a desktop, after Select on a phone. */
const checkboxes = computed(() => !phone.value || selection.checking.value)

function tap(id: string): void {
  if (selection.checking.value) {
    selection.toggle(id)
  } else if (phone.value) {
    emit('open', id)
  }
}

function restoreChecked(): void {
  emit('restoreChecked', checked.value.map((conversation) => conversation.id))
  selection.check(false)
}

// The dialog keeps the conversations it asks about while it closes.
const deleting = ref<SettingsArchivedConversation[]>([])
const deleteOpen = ref(false)

function askDelete(): void {
  deleting.value = checked.value
  deleteOpen.value = true
}

function confirmDelete(): void {
  deleteOpen.value = false
  emit('delete', deleting.value.map((conversation) => conversation.id))
  selection.check(false)
}
</script>

<template>
  <SettingsPage
    :title="t('archived.title')"
    :description="t('archived.description')"
  >
    <div class="flex flex-col gap-2">
      <!-- The title's line holds the count or, while rows are checked, what to do with them, at the same height. -->
      <div class="flex h-8 select-none items-center gap-2">
        <template v-if="checked.length || selection.checking.value">
          <span class="min-w-0 truncate text-[15px] font-medium text-fg-emphasis">{{ t('archived.selected', { count: checked.length }) }}</span>
          <span class="ml-auto flex shrink-0 gap-1.5">
            <Button size="sm" :disabled="!checked.length" @click="restoreChecked">{{ t('archived.restore') }}</Button>
            <Button size="sm" variant="danger" :disabled="!checked.length" @click="askDelete">{{ t('archived.delete') }}</Button>
            <Button v-if="phone" size="sm" variant="ghost" @click="selection.check(false)">{{ uiText('common.done') }}</Button>
          </span>
        </template>
        <template v-else>
          <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('archived.count', { count: conversations.length }, conversations.length) }}</h3>
          <Button v-if="phone && conversations.length" size="sm" variant="ghost" class="ml-auto" @click="selection.check(true)">{{ t('archived.select') }}</Button>
        </template>
      </div>

      <div class="overflow-hidden rounded-xl border border-line bg-surface-float">
        <div class="flex h-11 items-center gap-3 border-b border-line px-3">
          <Checkbox
            v-if="checkboxes"
            :model-value="allChecked"
            :partial="someChecked"
            :disabled="!shown.length"
            :label="t('archived.selectAll')"
            class="[&>span:last-child]:sr-only"
            @update:model-value="selection.toggleAll()"
          />
          <div class="min-w-0 flex-1">
            <TextInput
              v-model="query"
              search
              bare
              :placeholder="t('archived.search')"
              :aria-label="t('archived.searchLabel')"
            >
              <template #prefix><Search :size="ICON_PX.in24" /></template>
            </TextInput>
          </div>
        </div>
        <AsyncRegion
          :state="load"
          :label="t('archived.loading')"
          @retry="emit('retry')"
        >
          <div
            v-for="conversation in shown"
            :key="conversation.id"
            class="group flex cursor-default items-center gap-3 border-b border-line px-3 last:border-b-0"
            :class="[
              phone ? 'min-h-row py-2' : 'h-10',
              selection.selected.value.has(conversation.id) ? 'bg-active' : 'hover:bg-hover',
            ]"
            @click="tap(conversation.id)"
          >
            <span v-if="checkboxes" class="flex" @click.stop>
              <Checkbox
                :model-value="selection.selected.value.has(conversation.id)"
                :label="t('archived.selectLabel', { title: conversationTitle(conversation.title) })"
                class="[&>span:last-child]:sr-only"
                @update:model-value="selection.toggle(conversation.id, $event)"
              />
            </span>
            <!-- A phone's narrow row gives the title its own line and puts the project and the
                 time under it, as the phone's own mail and file lists do. -->
            <span v-if="phone" class="flex min-w-0 flex-1 flex-col gap-0.5">
              <TruncatedText class="min-w-0 text-setting text-fg-emphasis" :text="conversationTitle(conversation.title)" />
              <span class="flex min-w-0 items-center gap-1.5 text-detail text-fg-subtle">
                <template v-if="conversation.project">
                  <Folder :size="ICON_PX.in24" class="shrink-0" aria-hidden="true" />
                  <TruncatedText class="min-w-0" :text="conversation.project" />
                  <span aria-hidden="true">·</span>
                </template>
                <span class="shrink-0 whitespace-nowrap tabular-nums">{{ formatModified(conversation.updatedAt) }}</span>
              </span>
            </span>
            <TruncatedText v-else class="min-w-0 flex-1 text-setting text-fg-emphasis" :text="conversationTitle(conversation.title)" />
            <!-- The project and the time, in columns of their own width so they line up from row to
                 row; the row's buttons take their room under the pointer or the focus. -->
            <span v-if="!phone" class="relative flex shrink-0 items-center justify-end gap-3">
              <span class="flex w-36 min-w-0 items-center gap-1.5 text-detail text-fg-subtle group-hover:invisible group-focus-within:invisible">
                <template v-if="conversation.project">
                  <Folder :size="ICON_PX.in24" class="shrink-0" aria-hidden="true" />
                  <TruncatedText class="min-w-0" :text="conversation.project" />
                </template>
              </span>
              <span class="w-44 whitespace-nowrap text-right text-detail tabular-nums text-fg-subtle group-hover:invisible group-focus-within:invisible">
                {{ formatModified(conversation.updatedAt) }}
              </span>
              <span class="invisible absolute right-0 flex gap-1.5 group-hover:visible group-focus-within:visible">
                <Button size="sm" variant="ghost" @click="emit('open', conversation.id)">{{ t('archived.open') }}</Button>
                <Button size="sm" @click="emit('restore', conversation.id)">{{ t('archived.restore') }}</Button>
              </span>
            </span>
          </div>
          <div
            v-if="!shown.length"
            class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
          >
            {{ query ? t('archived.noResults') : t('archived.none') }}
          </div>
        </AsyncRegion>
      </div>
    </div>
    <ConversationDeleteDialog
      :is-open="deleteOpen"
      :overlay-store="overlayStore"
      :titles="deleting.map((conversation) => conversationTitle(conversation.title))"
      @close="deleteOpen = false"
      @delete="confirmDelete"
    />
  </SettingsPage>
</template>

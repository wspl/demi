<script setup lang="ts">
import { computed, nextTick, ref, useId, watch } from 'vue'
import { Search, SearchX } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from '../ui/Dialog.vue'
import HighlightText from '../ui/HighlightText.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import Tag from '../ui/Tag.vue'
import TextInput from '../ui/TextInput.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { SEARCH_QUERY_MAX, useConversationSearch, type SearchRow, type SearchSource } from './search'

/**
 * The search window (`product.md` § Finding a conversation), which ⌘K and
 * Search in the sidebar open, as ⌘K does in Slack, Linear and ChatGPT: a
 * field, and under it the most recent conversations until the user types,
 * then, once typing pauses, the conversations that match. Each row shows the
 * title and the line of the message that matched, the matches marked in
 * both, when the conversation was last active, and Archived for an archived
 * one. The window is as tall as its list, up to a limit past which the list
 * scrolls, and hangs from the top, so the field stays put as the list
 * changes, as in Spotlight and Raycast. ↑ and
 * ↓ move the selection, Return or a click opens the selected conversation,
 * and Escape closes the window. An input method's Return picks its
 * candidate and opens nothing.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The most recent conversations, which the window lists while its field is empty. */
  recent: SearchRow[]
  search: SearchSource
}>()
const emit = defineEmits<{
  close: []
  /** Opens the conversation, at the message that matched when there is one. */
  open: [row: SearchRow]
}>()

const { query, view, retry, clear } = useConversationSearch(() => props.search, () => props.recent)
const rows = computed(() => (view.value.kind === 'failed' ? [] : view.value.rows))
const selected = ref(0)
const listId = useId()
const list = ref<HTMLElement>()

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      clear()
      selected.value = 0
    }
  },
)
// Another list starts at its first row: the recent conversations, or the results of another query.
// A list that only changes, as the recent ones do while a conversation runs, keeps the selection.
watch(
  () => (view.value.kind === 'results' ? `results ${view.value.query}` : view.value.kind),
  () => {
    selected.value = 0
  },
)
/** The selected row, never past the end of a list that shrank. */
const current = computed(() => Math.min(selected.value, Math.max(0, rows.value.length - 1)))

function move(step: number): void {
  if (!rows.value.length) {
    return
  }
  selected.value = (current.value + step + rows.value.length) % rows.value.length
  void nextTick(() => {
    list.value
      ?.querySelector(`[data-row="${selected.value}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  })
}

function openRow(row: SearchRow | undefined): void {
  if (row) {
    emit('open', row)
  }
}

// The field's keys; TextInput keeps an input method's keys to itself.
function keydown(event: KeyboardEvent): void {
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    move(event.key === 'ArrowDown' ? 1 : -1)
  } else if (event.key === 'Enter') {
    event.preventDefault()
    openRow(rows.value[current.value])
  } else if (event.key === 'Escape') {
    // The window closes itself, in a specimen too, where the dialog leaves
    // Escape to the page.
    event.preventDefault()
    emit('close')
  }
}

const emptyLabel = computed(() =>
  view.value.kind === 'recent' ? 'No conversations yet.' : 'No conversations match.',
)
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="wide"
    anchor="top"
    label="Search"
    :scroll-content="false"
    @close="emit('close')"
  >
    <!-- As tall as its list, up to a limit past which the list scrolls; the dialog hangs from
         the top, so the field stays where it is as the list grows and shrinks. -->
    <div class="flex max-h-[min(32rem,76vh)] min-h-0 flex-col">
      <!-- The field's 28px row sits 12px from the top, centered on the close button beside it. -->
      <div class="shrink-0 p-3 pr-12">
        <TextInput
          v-model="query"
          placeholder="Search conversations"
          aria-label="Search conversations"
          role="combobox"
          aria-autocomplete="list"
          :aria-controls="listId"
          :aria-expanded="rows.length > 0"
          :aria-activedescendant="rows.length ? `${listId}-${current}` : undefined"
          :maxlength="SEARCH_QUERY_MAX"
          focused
          @keydown="keydown"
        >
          <template #prefix><Search :size="ICON_PX.in24" /></template>
        </TextInput>
      </div>
      <ScrollArea class="min-h-0 border-t border-line" viewport-class="flex flex-col p-2">
        <RegionStatus
          v-if="view.kind === 'failed'"
          status="failed"
          label="Could not search."
          :detail="view.detail"
          :on-retry="retry"
        />
        <RegionStatus
          v-else-if="view.kind === 'searching' && !rows.length"
          status="loading"
          label="Searching…"
        />
        <RegionStatus
          v-else-if="!rows.length"
          status="note"
          :icon="SearchX"
          :label="emptyLabel"
        />
        <div
          v-else
          :id="listId"
          ref="list"
          role="listbox"
          aria-label="Conversations"
          class="flex flex-col gap-px"
        >
          <div
            v-for="(row, index) in rows"
            :id="`${listId}-${index}`"
            :key="row.conversationId"
            role="option"
            :aria-selected="index === current"
            :data-row="index"
            class="flex cursor-default select-none flex-col gap-0.5 rounded-md px-2.5 py-2"
            :class="index === current ? 'bg-active' : ''"
            @mousemove="selected = index"
            @click="openRow(row)"
          >
            <div class="flex min-w-0 items-center gap-2">
              <span class="min-w-0 flex-1 truncate text-chrome text-fg-emphasis"><HighlightText :text="row.title" :ranges="row.titleRanges" /></span>
              <Tag v-if="row.archived">Archived</Tag>
              <RelativeTime class="shrink-0 text-[12px] text-fg-subtle" :timestamp="row.lastActiveAt" />
            </div>
            <!-- Two lines hold the backend's 160 characters, so the marked words stay in sight. -->
            <p v-if="row.match" class="line-clamp-2 break-words text-[13px] leading-5 text-fg-muted">
              <HighlightText :text="row.match.text" :ranges="row.match.ranges" />
            </p>
          </div>
        </div>
      </ScrollArea>
    </div>
  </Dialog>
</template>

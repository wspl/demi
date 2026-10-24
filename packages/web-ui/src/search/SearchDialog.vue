<script setup lang="ts">
import { t } from './locales/text'
import { computed, ref, useId, watch } from 'vue'
import { Search, SearchX, X } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { revealRow, stepHighlight } from '../composables/list-highlight'
import Dialog from '../ui/Dialog.vue'
import HighlightText from '../ui/HighlightText.vue'
import { conversationTitle } from '../agent/conversation-title'
import RegionStatus from '../ui/RegionStatus.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import Tag from '../ui/Tag.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { formatCount } from '../ui/numbers'
import { useConversationSearch, type MessageRow, type MessageSource, type SearchRow, type SearchSource } from './search'

/**
 * The search window, which ⌘K and Search in the sidebar open, as ⌘K does in
 * Slack, Linear and ChatGPT.
 *
 * - It is a field, and under it the most recent conversations until the user
 *   types, then, once typing pauses, the conversations that match.
 * - Each row shows the title and the line of the message that matched, the
 *   matches marked in both, when the conversation was last active, and Archived
 *   for an archived one.
 * - The window is as tall as its list, up to a limit past which the list
 *   scrolls, and hangs from the top, so the field stays put as the list changes,
 *   as in Spotlight and Raycast.
 * - ↑ and ↓ move the selection, Return or a click opens the selected
 *   conversation, and Escape closes the window.
 * - An input method's Return picks its candidate and opens nothing.
 * - Opened with `searchIn`, as Find opens it in a conversation, the field
 *   starts with This conversation, and the window lists the messages of the
 *   shown conversation that match, newest first, each with who wrote it and
 *   when, as Slack's ⌘F starts its search in the channel shown. Return or a
 *   click opens the conversation at the message. The token's ✕, or Backspace
 *   at the start of the field, removes it, and the same text searches every
 *   conversation.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The most recent conversations, which the window lists while its field is empty. */
  recent: SearchRow[]
  search: SearchSource
  /** The search of the shown conversation's messages, which the window opens in; none to open searching every conversation. */
  searchIn?: MessageSource | null
}>()
const emit = defineEmits<{
  close: []
  /** Opens the conversation, at the message that matched when there is one. */
  open: [row: SearchRow]
  /** Opens the shown conversation at the message. */
  openMessage: [row: MessageRow]
}>()

/** Whether the field holds This conversation, and the window searches in it. */
const scoped = ref(false)
const { query, view, retry, clear } = useConversationSearch(
  () => props.search,
  () => props.recent,
  () => (scoped.value ? props.searchIn ?? null : null),
)

/** A row of the list: a conversation, or a message of the shown one. */
type Row = { kind: 'conversation'; row: SearchRow } | { kind: 'message'; row: MessageRow }
const found = computed(() => (view.value.kind === 'results' || view.value.kind === 'searching' ? view.value.found : null))
const rows = computed<Row[]>(() => {
  if (view.value.kind === 'recent')
    return view.value.rows.map((row) => ({ kind: 'conversation', row }))
  const now = found.value
  if (now?.kind === 'messages')
    return now.rows.map((row) => ({ kind: 'message', row }))
  return now?.rows.map((row) => ({ kind: 'conversation', row })) ?? []
})
/** More messages matched than the list holds. */
const limited = computed(() => found.value?.kind === 'messages' && found.value.limited)
const selected = ref(0)
const listId = useId()
const list = ref<HTMLElement>()
const field = ref<InstanceType<typeof TextInput>>()

watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      scoped.value = props.searchIn != null
      clear()
      selected.value = 0
    }
  },
  // A window open from the start, as a pinned specimen is, opens with the token too.
  { immediate: true },
)
// Find or ⌘K while the window is open opens it again, in the conversation or in every one.
watch(
  () => props.searchIn,
  (searchIn) => {
    if (props.isOpen) {
      scoped.value = searchIn != null
    }
  },
)
// Another list starts at its first row: the recent conversations, or the results of another query.
// A list that only changes, as the recent ones do while a conversation runs, keeps the selection.
watch(
  [
    () => view.value.kind,
    () => (view.value.kind === 'results' ? view.value.query : null),
    () => (view.value.kind === 'results' ? view.value.scope : null),
  ],
  () => {
    selected.value = 0
  },
)

/** Removes This conversation: the same text searches every conversation. */
function removeScope(): void {
  scoped.value = false
  field.value?.focus()
}
/** The selected row, never past the end of a list that shrank. */
const current = computed(() => Math.min(selected.value, Math.max(0, rows.value.length - 1)))

function move(step: 1 | -1): void {
  if (!rows.value.length) {
    return
  }
  selected.value = stepHighlight(current.value, step, rows.value.length)
  revealRow(list.value, '[role=option]', selected.value)
}

function openRow(row: Row | undefined): void {
  if (row?.kind === 'conversation') {
    emit('open', row.row)
  } else if (row) {
    emit('openMessage', row.row)
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
  } else if (event.key === 'Backspace' && scoped.value && atStart(event.target)) {
    event.preventDefault()
    removeScope()
  } else if (event.key === 'Escape') {
    // The window closes itself, in a specimen too, where the dialog leaves
    // Escape to the page.
    event.preventDefault()
    emit('close')
  }
}

/** Whether the caret stands at the start of the field, with nothing selected. */
function atStart(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement && target.selectionStart === 0 && target.selectionEnd === 0
}

const emptyLabel = computed(() => {
  if (view.value.kind === 'recent')
    return t('noConversations')
  return view.value.kind === 'typeToSearch' ? t('typeToSearch') : t('noResults')
})
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="wide"
    anchor="top"
    :label="t('title')"
    :scroll-content="false"
    @close="emit('close')"
  >
    <!-- As tall as its list, up to a limit past which the list scrolls; the dialog hangs from
         the top, so the field stays where it is as the list grows and shrinks. -->
    <div class="flex max-h-[min(32rem,76vh)] min-h-0 flex-col">
      <!-- The field's 28px row sits 12px from the top, centered on the close button beside it. -->
      <div class="shrink-0 p-3 pr-12">
        <TextInput
          ref="field"
          v-model="query"
          search
          :placeholder="scoped ? t('placeholderIn') : t('placeholder')"
          :aria-label="scoped ? t('placeholderIn') : t('placeholder')"
          role="combobox"
          aria-autocomplete="list"
          :aria-controls="listId"
          :aria-expanded="rows.length > 0"
          :aria-activedescendant="rows.length ? `${listId}-${current}` : undefined"
          focused
          asked
          @keydown="keydown"
        >
          <template #prefix>
            <Search :size="ICON_PX.in24" />
            <!-- The scope, a token before the text, as Slack's in: and Discord's filters stand. -->
            <span
              v-if="scoped"
              class="ml-1.5 inline-flex h-5 shrink-0 select-none items-center gap-0.5 rounded bg-hover pl-1.5 text-caption font-medium text-fg-muted phone:h-6"
              data-search-scope
            >
              {{ t('scope') }}
              <Tooltip :content="t('scopeRemove')">
                <button
                  type="button"
                  class="flex size-5 cursor-default items-center justify-center rounded text-fg-subtle hover:bg-hover hover:text-fg phone:size-6"
                  :aria-label="t('scopeRemove')"
                  @click.stop="removeScope"
                >
                  <X :size="12" />
                </button>
              </Tooltip>
            </span>
          </template>
        </TextInput>
      </div>
      <ScrollArea class="min-h-0 border-t border-line" viewport-class="flex flex-col p-2">
        <RegionStatus
          v-if="view.kind === 'failed'"
          status="failed"
          :label="t('failed')"
          :detail="view.detail"
          :on-retry="retry"
        />
        <RegionStatus
          v-else-if="view.kind === 'searching' && !rows.length"
          status="loading"
          :label="t('searching')"
        />
        <RegionStatus
          v-else-if="!rows.length"
          status="note"
          :icon="view.kind === 'typeToSearch' ? Search : SearchX"
          :label="emptyLabel"
        />
        <div
          v-else
          :id="listId"
          ref="list"
          role="listbox"
          :aria-label="scoped ? t('messages') : t('list')"
          class="flex flex-col gap-px"
        >
          <div
            v-for="(item, index) in rows"
            :id="`${listId}-${index}`"
            :key="item.kind === 'conversation' ? item.row.conversationId : item.row.blockId"
            role="option"
            :aria-selected="index === current"
            class="flex cursor-default select-none flex-col gap-0.5 rounded-md px-2.5 py-2 phone:min-h-row phone:justify-center phone:py-2.5"
            :class="index === current ? 'bg-active' : ''"
            @mousemove="selected = index"
            @click="openRow(item)"
          >
            <template v-if="item.kind === 'conversation'">
              <div class="flex min-w-0 items-center gap-2">
                <span class="min-w-0 flex-1 truncate text-chrome text-fg-emphasis"><HighlightText :text="conversationTitle(item.row.title)" :ranges="item.row.titleRanges" /></span>
                <Tag v-if="item.row.archived">{{ t('archived') }}</Tag>
                <RelativeTime class="shrink-0 text-detail text-fg-subtle" :timestamp="item.row.lastActiveAt" />
              </div>
              <!-- Two lines hold the backend's 160 characters, so the marked words stay in sight. -->
              <p v-if="item.row.match" class="line-clamp-2 break-words text-explanation leading-5 text-fg-muted">
                <HighlightText :text="item.row.match.text" :ranges="item.row.match.ranges" />
              </p>
            </template>
            <template v-else>
              <div class="flex min-w-0 items-center gap-2">
                <span class="min-w-0 flex-1 truncate text-chrome text-fg-emphasis">{{ item.row.author === 'user' ? t('you') : t('agent') }}</span>
                <RelativeTime class="shrink-0 text-detail text-fg-subtle" :timestamp="item.row.time" />
              </div>
              <p class="line-clamp-2 break-words text-explanation leading-5 text-fg-muted">
                <HighlightText :text="item.row.text" :ranges="item.row.ranges" />
              </p>
            </template>
          </div>
        </div>
        <p v-if="limited && rows.length" class="px-2.5 pb-1 pt-2 text-detail text-fg-subtle">
          {{ t('limited', { count: formatCount(rows.length) }, rows.length) }}
        </p>
      </ScrollArea>
    </div>
  </Dialog>
</template>

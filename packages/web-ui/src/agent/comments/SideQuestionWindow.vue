<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { MessageCircleQuestionMark } from '@lucide/vue'
import { ICON_PX } from '../../ui/icon-metrics'
import AgentMessageList from '../AgentMessageList.vue'
import SessionOverlay from '../SessionOverlay.vue'
import TabItem from '../TabItem.vue'
import { useFocusReturn } from '../../overlay/focusReturn'
import { focusByPage } from '../../ui/focus-by-page'
import type { PersistedScrollState } from '../../composables/useBlockVirtualizer'
import { pageToRead, readKey, shownTurns, type ShownSideQuestion } from './asks'

/**
 * A side question's conversation, its steps included, in the window a
 * subagent's opens in: its own turns only, read-only, the first drawn as the
 * user's question with its quote; full screen on a phone. The history it
 * started from is the conversation's own, and its instruction is the product's.
 */
const props = defineProps<{
  /** The side question shown; null closes the window. */
  question: ShownSideQuestion | null
}>()

const emit = defineEmits<{
  close: []
  /** Read the page before the window shown, as the reader nears its start. */
  read: [id: string, at: 'latest' | 'before']
}>()

const blocks = computed(() => (props.question ? shownTurns(props.question) : []))

/**
 * Where the list opens: at its start, the user's first question and its quote
 * in view, since a side conversation is short and starts with what the user
 * asked. A taller one the user scrolls down. The list places itself once, when
 * it first has rows, so it is drawn once its first question is held.
 */
const opening = computed<PersistedScrollState | undefined>(() => {
  const first = blocks.value[0]
  return first && { anchor: { blockId: first.id, anchorIndex: 0, offsetPx: 0, scrollTop: 0 }, heightCache: new Map() }
})

// The window takes the focus as it opens, as a window macOS opens does, so its
// Escape is a key typed in it. Closed, it gives the focus back to the card's ↗
// as the page, drawing no ring, which a key press would otherwise make the
// browser draw.
const body = ref<HTMLElement | null>(null)
/**
 * The window's region while it is open, which still holds the focus as it
 * closes.
 */
let region: HTMLElement | null = null
const focusReturn = useFocusReturn((active) => region?.contains(active) === true)
watch(body, (shown, was) => {
  if (shown && !was) {
    focusReturn.open()
    region = shown.closest<HTMLElement>('[data-region]')
    if (region)
      focusByPage(region, { preventScroll: true })
  } else if (!shown && was) {
    focusReturn.close()
    region = null
  }
}, { flush: 'post' })
onBeforeUnmount(() => focusReturn.close())

// Its turns are read until the first question is held, as the card reads them.
watch(
  () => props.question && readKey(props.question),
  (key) => {
    const page = props.question && pageToRead(props.question)
    if (key && page)
      emit('read', props.question!.id, page)
  },
  { immediate: true },
)
</script>

<template>
  <SessionOverlay :open="question !== null" @close="emit('close')">
    <template #tabs>
      <TabItem v-if="question" :title="question.title ?? question.question" is-active :closable="false">
        <template #mark>
          <MessageCircleQuestionMark :size="ICON_PX.in28" class="text-[var(--ask-ink)]" aria-hidden="true" />
        </template>
      </TabItem>
    </template>
    <div v-if="question" ref="body" class="h-full">
    <AgentMessageList
      v-if="opening"
      :key="question.id"
      class="h-full"
      :conversation-id="question.id"
      :node="question.id"
      :blocks="blocks"
      :at-start="true"
      :at-end="true"
      :failures="question.failures"
      :pending-steers="[]"
      :queue="[]"
      :phase="question.working ? 'running' : 'idle'"
      :bottom-offset="16"
      :persisted-scroll-state="opening"
      read-only
    />
    </div>
  </SessionOverlay>
</template>

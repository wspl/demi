<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { ArrowUpRight, MessageCircleQuestionMark, X } from '@lucide/vue'
import type { Block, ProviderFailureFacts } from '@demicodes/protocol'
import Button from '../../ui/Button.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import IconButton from '../../ui/IconButton.vue'
import SendField from '../../ui/SendField.vue'
import Shimmer from '../../ui/Shimmer.vue'
import StreamedMarkdown from '../../ui/StreamedMarkdown.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { vContextMenu, type ContextMenuDeclaration } from '../../ui/context-menu'
import ErrorBlock from '../blocks/ErrorBlock.vue'
import { t } from '../locales/text'
import type { CardTurn } from './asks'

/**
 * A side question's card, set into the reply after the block it asks about.
 *
 * - Each question is in bold, with its short answer.
 * - While the question works, one status line stands in place of the steps. A
 *   failure shows its record with Retry.
 * - × at the card's corner folds it into the margin. Open Full Conversation,
 *   beside it, shows every step in the subagent window.
 * - Reply opens a field for a follow-up.
 *
 * Only the header's icon is purple.
 */
const props = defineProps<{
  /** The small model's title; null until it has one, when the card says Ask. */
  title: string | null
  turns: readonly CardTurn[]
  /** The record the last turn failed with, and what the backend knows of it. */
  failure?: Extract<Block, { type: 'error' }> | null
  failureFacts?: ProviderFailureFacts | null
}>()

/**
 * Reply's field is open, and what it holds: the owner keeps them, so a pinned
 * specimen can open it.
 */
const replying = defineModel<boolean>('replying', { default: false })
const reply = defineModel<string>('reply', { default: '' })

const emit = defineEmits<{
  ask: [question: string]
  retry: []
  collapse: []
  openFull: []
}>()

const root = ref<HTMLElement>()

/**
 * The last turn is still made, its status shown or its answer streaming: Reply
 * comes back with the answer.
 */
const working = computed(() => {
  const last = props.turns.at(-1)
  return last !== undefined && (last.status !== null || last.streaming)
})

function send(text: string): void {
  replying.value = false
  emit('ask', text)
}

function cancel(): void {
  replying.value = false
  reply.value = ''
}

/**
 * An empty reply field closes when a press lands outside the card. A press on
 * one of the card's controls does that control's action, and typed text stays.
 * A press, not the field's blur, decides, since the blur comes before the click
 * it would undo.
 */
function onPointerDown(event: PointerEvent): void {
  if (replying.value && reply.value.trim() === '' && !root.value?.contains(event.target as Node))
    replying.value = false
}
onMounted(() => document.addEventListener('pointerdown', onPointerDown, { capture: true }))
onBeforeUnmount(() => document.removeEventListener('pointerdown', onPointerDown, { capture: true }))

// The card is a place of its own in the reply: text selected in it is copied,
// never asked about or commented on again.
const menu: ContextMenuDeclaration = { entries: () => [], text: true, selectionEntries: () => [] }

defineExpose({ element: root, failed: () => props.failure != null })
</script>

<template>
  <div
    ref="root"
    v-context-menu="menu"
    tabindex="-1"
    data-ask-card
    data-no-anchor
    class="ask-card rounded-xl border border-[var(--ask-border)] p-3 text-conversation outline-none focus-visible:ring-2 focus-visible:ring-[var(--ask-border)]"
  >
    <!--
      The header: what the side conversation is about, and its two actions.
    -->
    <div class="-mr-1.5 -mt-1.5 mb-1.5 flex items-center gap-1.5">
      <MessageCircleQuestionMark :size="ICON_PX.in24" class="shrink-0 text-[var(--ask-ink)]" aria-hidden="true" />
      <span class="min-w-0 flex-1 truncate text-detail text-fg-muted">{{ title ?? t('ask.ask') }}</span>
      <Tooltip :content="t('ask.openFull')">
        <IconButton :icon="ArrowUpRight" size="sm" variant="ghost" :aria-label="t('ask.openFull')" @click="emit('openFull')" />
      </Tooltip>
      <Tooltip :content="t('ask.collapse')">
        <IconButton :icon="X" size="sm" variant="ghost" :aria-label="t('ask.collapse')" @click="emit('collapse')" />
      </Tooltip>
    </div>
    <div class="flex flex-col gap-3">
      <div v-for="(turn, index) in turns" :key="index" class="flex flex-col gap-1">
        <!--
          A div, not a p: the reply around the card spaces its own paragraphs.
        -->
        <div class="whitespace-pre-wrap font-semibold text-fg-emphasis">{{ turn.question }}</div>
        <Shimmer v-if="turn.status !== null" active class="truncate text-fg-muted">{{ turn.status }}</Shimmer>
        <StreamedMarkdown v-else-if="turn.answer !== ''" :content="turn.answer" :pictures="turn.pictures" :streaming="turn.streaming" class="text-fg-body" />
      </div>
      <div v-if="failure" class="flex flex-col items-start gap-2">
        <ErrorBlock
          class="w-full"
          :message="failure.message"
          :code="failure.code"
          :diagnostics="failure.diagnostics"
          :provider-id="failure.model.providerId"
          :retry-at="failureFacts?.retryAt ?? null"
          :signed-out="failureFacts?.signedOut ?? null"
          :not-exposed="failureFacts?.notExposed ?? null"
          :device-offline="failureFacts?.deviceOffline ?? null"
        />
        <Button size="xs" @click="emit('retry')">{{ t('ask.retry') }}</Button>
      </div>
      <SendField
        v-if="replying"
        v-model="reply"
        :placeholder="t('ask.followUpPlaceholder')"
        :send-label="t('ask.reply')"
        focus
        @send="send"
        @cancel="cancel"
      />
    </div>
    <div v-if="!replying && !failure && !working" class="mt-2.5 flex items-center">
      <!--
        Its label reads at the card's size, the reply's, as the question and
        answer do.
      -->
      <Button size="sm" @click="replying = true"><span class="text-conversation">{{ t('ask.reply') }}</span></Button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { Check, MessageCircleMore, Pencil, Trash2 } from '@lucide/vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import CommentBox from './CommentBox.vue'
import CommentQuote from './CommentQuote.vue'

/**
 * A comment: its text, read as text, with ghost Delete and Edit at the row's
 * right end; Edit turns the row into the editor. A sent comment stays as it was
 * sent and says so.
 *
 * In the reply, under the line of its words (`inline`), it is a box with a
 * thin border, as a side question's card, led by the comment's yellow mark; a
 * sent one is dimmer. In the message field's list it is a row under the words
 * it quotes.
 */
const props = defineProps<{
  text: string
  sent: boolean
  /**
   * The text it is about, shown above it where the list stands away from the
   * reply.
   */
  quote?: string
  /** It stands in the reply, under the line of its words. */
  inline?: boolean
}>()

const emit = defineEmits<{
  save: [text: string]
  delete: []
}>()

const editing = ref(false)

function save(text: string): void {
  editing.value = false
  emit('save', text)
}
</script>

<template>
  <div
    class="flex flex-col gap-1.5"
    :class="inline ? 'rounded-xl border border-line py-1.5 pl-3 pr-1.5 text-conversation-secondary' : ''"
    :data-comment-sent="sent || undefined"
    data-comment
  >
    <CommentQuote v-if="quote" :text="quote" />
    <CommentBox v-if="editing" :text="text" @save="save" @cancel="editing = false" />
    <div v-else class="flex items-start gap-2">
      <span v-if="inline" class="inline-flex h-7 shrink-0 items-center text-[var(--comment-ink)]" :class="sent ? 'opacity-60' : ''">
        <MessageCircleMore :size="ICON_PX.in28" aria-hidden="true" />
      </span>
      <!-- A div, not a p: in the reply, the reply's paragraphs' spacing would reach it. -->
      <div
        class="min-w-0 flex-1 whitespace-pre-wrap py-1"
        :class="[inline ? 'leading-5' : 'text-explanation', sent && inline ? 'text-fg-muted' : 'text-fg-body']"
      >{{ text }}</div>
      <span v-if="props.sent" class="inline-flex h-7 shrink-0 items-center gap-1 pr-1 text-detail text-fg-subtle">
        <Check :size="ICON_PX.in24" aria-hidden="true" />
        {{ t('comments.sent') }}
      </span>
      <div v-else class="flex shrink-0 items-center">
        <Tooltip :content="shared('common.delete')">
          <IconButton :icon="Trash2" size="sm" variant="ghost" :aria-label="shared('common.delete')" @click="emit('delete')" />
        </Tooltip>
        <Tooltip :content="t('transcript.user.edit')">
          <IconButton :icon="Pencil" size="sm" variant="ghost" :aria-label="t('transcript.user.edit')" @click="editing = true" />
        </Tooltip>
      </div>
    </div>
  </div>
</template>

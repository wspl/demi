<script setup lang="ts">
import { MessageCircleMore, X } from '@lucide/vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import { t } from '../locales/text'
import { useCommentRevealer, type CommentPlace } from './comments'

/**
 * The comments waiting in the draft, as the composer shows them by its
 * paperclip: a round-ended pill as tall as the composer's own buttons, the
 * comment's mark, *2 Comments* and a ghost ×, which removes them all.
 *
 * A click on the pill brings the next comment's box into view and marks it,
 * in the order the conversation reads them, from the first again after the
 * last, as a find bar's Return steps through its matches. The boxes in the
 * reply hold Edit and Delete. The composer's `carried` slot holds the pill.
 */
const props = defineProps<{
  /** The comments, in the order the conversation reads them. */
  comments: readonly CommentPlace[]
  /** They cannot be changed, as an edit's while it is sent: the pill has no ×. */
  readonly?: boolean
}>()

const emit = defineEmits<{
  clear: []
}>()

const reveal = useCommentRevealer()
/** The comment the pill showed last; the next click shows the one after it. */
let shown: string | null = null

function next(): void {
  const at = props.comments.findIndex((comment) => comment.id === shown)
  const comment = props.comments[at < 0 ? 0 : (at + 1) % props.comments.length]
  if (!comment)
    return
  shown = comment.id
  reveal?.(comment)
}
</script>

<template>
  <span data-waiting-comments class="btn-solid inline-flex h-chip select-none items-center rounded-full text-chrome text-fg-body">
    <Tooltip :content="t('comments.next')">
      <button
        type="button"
        class="inline-flex h-full items-center gap-1.5 rounded-l-full pl-2.5 hover:text-fg-emphasis"
        :class="readonly ? 'rounded-r-full pr-2.5' : 'pr-1.5'"
        @mousedown.prevent
        @click="next"
      >
        <MessageCircleMore :size="ICON_PX.in24" class="text-[var(--comment-ink)]" aria-hidden="true" />
        {{ t('comments.count', { count: comments.length }, comments.length) }}
      </button>
    </Tooltip>
    <Tooltip v-if="!readonly" :content="t('comments.remove', {}, comments.length)">
      <IconButton
        :icon="X"
        size="xs"
        variant="ghost"
        class="mr-1"
        :aria-label="t('comments.remove', {}, comments.length)"
        @mousedown.prevent
        @click="emit('clear')"
      />
    </Tooltip>
  </span>
</template>

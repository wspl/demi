<script setup lang="ts">
import { ref } from 'vue'
import { Check, MessageCircleMore, Pencil, Trash2 } from '@lucide/vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import CommentBox from './CommentBox.vue'
import SentMessage from '../message-editor/SentMessage.vue'
import type { MessageCapsule } from '../message-editor/capsules'
import type { DraftMessage, MessageFiles } from './comments'

/**
 * A comment: its message, shown as a sent message is, as typed with its
 * Markdown highlighted and a capsule for each file, with ghost Delete and Edit
 * at the row's right end; Edit turns the row into the editor. A sent comment
 * stays as it was sent and says so; while an edit of its message is open,
 * it takes Delete alone.
 *
 * It stands in the reply, under the line of its words: a box with a thin
 * border, as a side question's card, led by the comment's yellow mark; a sent
 * one is dimmer.
 */
const props = defineProps<{
  /** Its message's Markdown, an attachment mark where each capsule stands. */
  markdown: string
  /** Its files, in the order of their marks. */
  attachments: readonly MessageCapsule[]
  /** The message Edit opens: a waiting comment's; none for a sent one. */
  editingMessage?: DraftMessage
  /** The files its editor takes, as the composer takes them. */
  files?: MessageFiles
  sent: boolean
  /** A sent comment an open edit of its message holds: Delete takes it out of the edit. */
  deletable?: boolean
}>()

const emit = defineEmits<{
  save: [message: DraftMessage]
  delete: []
}>()

const editing = ref(false)

function save(message: DraftMessage): void {
  editing.value = false
  emit('save', message)
}
</script>

<template>
  <!-- While it reads, the text starts further in than the icons end, whose own
       hit area pads them; the box being edited is the same distance from
       every side. -->
  <div
    class="flex flex-col gap-1.5 [--found-inset-x:0px] [--found-radius:var(--radius-xl)]"
    :class="['rounded-xl border border-line py-1.5 text-conversation-secondary', editing ? 'px-1.5' : 'pl-3 pr-1.5']"
    :data-comment-sent="sent || undefined"
    data-comment
  >
    <CommentBox v-if="editing && editingMessage && files" :message="editingMessage" :files="files" @save="save" @cancel="editing = false" />
    <div v-else class="flex items-start gap-2">
      <span class="inline-flex h-7 shrink-0 items-center text-[var(--comment-ink)]" :class="sent ? 'opacity-60' : ''">
        <MessageCircleMore :size="ICON_PX.in28" aria-hidden="true" />
      </span>
      <!-- A div, not a p: in the reply, the reply's paragraphs' spacing would reach it. -->
      <div
        class="min-w-0 flex-1 py-1 leading-5"
        :class="sent ? 'text-fg-muted' : 'text-fg-body'"
      >
        <SentMessage :markdown="markdown" :attachments="attachments" />
      </div>
      <span v-if="props.sent" class="inline-flex h-7 shrink-0 items-center gap-1 pr-1 text-detail text-fg-subtle">
        <Check :size="ICON_PX.in24" aria-hidden="true" />
        {{ t('comments.sent') }}
      </span>
      <!-- Its room stays while no edit is open, so Delete coming and going moves nothing. -->
      <Tooltip v-if="props.sent" :content="shared('common.delete')" :disabled="!deletable">
        <IconButton
          :icon="Trash2"
          size="sm"
          variant="ghost"
          :class="deletable ? '' : 'invisible'"
          :aria-label="shared('common.delete')"
          :aria-hidden="!deletable || undefined"
          :tabindex="deletable ? undefined : -1"
          @click="emit('delete')"
        />
      </Tooltip>
      <div v-if="!sent" class="flex shrink-0 items-center">
        <Tooltip :content="shared('common.delete')">
          <IconButton :icon="Trash2" size="sm" variant="ghost" :aria-label="shared('common.delete')" @click="emit('delete')" />
        </Tooltip>
        <Tooltip v-if="editingMessage && files" :content="t('transcript.user.edit')">
          <IconButton :icon="Pencil" size="sm" variant="ghost" :aria-label="t('transcript.user.edit')" @click="editing = true" />
        </Tooltip>
      </div>
    </div>
  </div>
</template>

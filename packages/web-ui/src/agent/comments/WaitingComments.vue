<script setup lang="ts">
import { ref } from 'vue'
import { MessageSquare, X } from '@lucide/vue'
import type { MessageComment } from '@demicodes/protocol'
import IconButton from '../../ui/IconButton.vue'
import Popover from '../../ui/Popover.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { ICON_PX } from '../../ui/icon-metrics'
import { appOverlayStore } from '../../overlay/appOverlay'
import { t } from '../locales/text'
import CommentItem from './CommentItem.vue'

/**
 * The comments waiting in the draft, as the composer shows them inside its box:
 * a round-ended pill as tall as the composer's own buttons, a comment mark, *2
 * Comments* and a ghost ×, which removes them all.
 *
 * A click on the pill lists them, each its quoted text and its comment, with
 * Edit and Delete: by the pill on the desktop, as a sheet on a phone. The
 * composer's `context` slot holds the pill.
 */
const props = defineProps<{
  /** The waiting comments, in the order the conversation reads them. */
  comments: readonly MessageComment[]
}>()

const emit = defineEmits<{
  edit: [id: string, text: string]
  remove: [id: string]
  clear: []
}>()

const open = ref(false)
const pill = ref<HTMLElement>()

function clear(): void {
  open.value = false
  emit('clear')
}

function remove(id: string): void {
  if (props.comments.length === 1)
    open.value = false
  emit('remove', id)
}
</script>

<template>
  <span ref="pill" data-waiting-comments class="btn-solid inline-flex h-chip select-none items-center rounded-full text-chrome text-fg-body">
    <button
      type="button"
      class="inline-flex h-full items-center gap-1.5 rounded-l-full pl-2.5 pr-1.5 hover:text-fg-emphasis"
      :aria-expanded="open"
      @mousedown.prevent
      @click="open = !open"
    >
      <MessageSquare :size="ICON_PX.in24" class="text-[var(--comment-ink)]" aria-hidden="true" />
      {{ t('comments.count', { count: comments.length }, comments.length) }}
    </button>
    <Tooltip :content="t('comments.remove', {}, comments.length)">
      <IconButton
        :icon="X"
        size="xs"
        variant="ghost"
        class="mr-1"
        :aria-label="t('comments.remove', {}, comments.length)"
        @mousedown.prevent
        @click="clear"
      />
    </Tooltip>
  </span>
  <!--
    The list opens above the pill, its left edge on the pill's, over the
    transcript.
  -->
  <Popover
    v-if="open && pill"
    is-open
    :overlay-store="appOverlayStore"
    :anchor-el="pill"
    side="top"
    sheet
    @close="open = false"
  >
    <!-- On a phone the list is a sheet's content, which drops this surface. -->
    <div role="dialog" :aria-label="t('comments.title')" class="overlay-shell flex max-h-[24rem] w-[340px] max-w-full flex-col gap-3 overflow-y-auto rounded-lg p-3">
      <CommentItem
        v-for="comment in comments"
        :key="comment.id"
        :text="comment.text"
        :sent="false"
        :quote="comment.quote"
        @save="emit('edit', comment.id, $event)"
        @delete="remove(comment.id)"
      />
    </div>
  </Popover>
</template>

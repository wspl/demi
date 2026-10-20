<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { isMacOS } from '@tiptap/core'
import Button from '../../ui/Button.vue'
import Tooltip from '../../ui/Tooltip.vue'
import { otherWayKeys } from '../send-way'
import { t } from '../locales/text'

/**
 * The box a new comment is written in under its words or its picture, as
 * GitHub's review offers *Add single comment* beside *Start a review*.
 *
 * It offers two buttons:
 *
 * - **Comment**, the default, adds the comment to the waiting comments.
 * - **Steer** sends a message holding the comment alone at once.
 *
 * Keys:
 *
 * - Return comments.
 * - ⌘Return (Control-Return elsewhere) sends now.
 * - Shift-Return starts a new line.
 * - Escape cancels.
 *
 * Without `sendNow` it has Comment alone.
 */
const props = defineProps<{
  sendNow: boolean
  /**
   * What was sent is being taken, as a picture being kept: nothing sends
   * meanwhile.
   */
  busy?: boolean
}>()

const text = defineModel<string>({ default: '' })

const emit = defineEmits<{
  comment: [text: string]
  sendNow: [text: string]
  cancel: []
}>()

const field = ref<HTMLTextAreaElement>()
const empty = computed(() => text.value.trim() === '')

onMounted(() => field.value?.focus({ preventScroll: true }))

// The text stays until the owner takes it and closes the box.
function done(way: 'comment' | 'sendNow'): void {
  if (empty.value || props.busy)
    return
  const written = text.value.trim()
  if (way === 'comment')
    emit('comment', written)
  else
    emit('sendNow', written)
}

function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing)
    return
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    // ⌘Return on macOS, Control-Return elsewhere, as the composer reads
    // Mod-Enter.
    const mod = isMacOS() ? event.metaKey : event.ctrlKey
    done(mod ? 'sendNow' : 'comment')
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('cancel')
  }
}
</script>

<template>
  <div data-comment-field class="overlay-shell flex flex-col gap-1.5 rounded-[14px] p-2">
    <textarea
      ref="field"
      v-model="text"
      rows="1"
      :placeholder="t('comments.placeholder')"
      class="max-h-32 min-h-7 resize-none bg-transparent px-1 py-1 text-chrome leading-5 text-fg outline-none [field-sizing:content] placeholder:text-fg-subtle"
      @keydown="onKeydown"
    />
    <div class="flex items-center justify-end gap-1.5">
      <Tooltip v-if="sendNow" :content="otherWayKeys()">
        <Button
          size="sm"
          :disabled="empty || busy"
          :disabled-reason="t('comments.writeFirst')"
          @mousedown.prevent
          @click="done('sendNow')"
        >
          {{ t('comments.sendNow') }}
        </Button>
      </Tooltip>
      <Button
        size="sm"
        variant="primary"
        :disabled="empty || busy"
        :disabled-reason="t('comments.writeFirst')"
        @mousedown.prevent
        @click="done('comment')"
      >
        {{ t('comments.comment') }}
      </Button>
    </div>
  </div>
</template>

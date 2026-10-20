<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { ArrowUp } from '@lucide/vue'
import IconButton from './IconButton.vue'
import Tooltip from './Tooltip.vue'
import type { PlaceholderText, TitleText } from './ui-text'

/**
 * A small field that sends what is typed, with the composer's send button at its
 * end, such as the box a new comment is written in.
 *
 * - Return sends, Shift-Return starts a new line, Escape cancels.
 * - It grows with its text up to a few lines.
 */
const props = defineProps<{
  placeholder: PlaceholderText
  /** The send button's name, such as Comment. */
  sendLabel: TitleText
  /** It takes the focus as it opens, on a phone too. */
  focus?: boolean
  /** It stands on its own over the page, on the overlay's surface, rather than in a card. */
  floating?: boolean
  /**
   * What was sent stays until the owner takes it, as a question about a
   * picture until the picture is kept: the owner empties the field, and it
   * sends nothing meanwhile.
   */
  holding?: boolean
}>()

const text = defineModel<string>({ default: '' })

const emit = defineEmits<{
  send: [text: string]
  cancel: []
}>()

const field = ref<HTMLTextAreaElement>()
const empty = computed(() => text.value.trim() === '')

onMounted(() => {
  if (props.focus)
    field.value?.focus({ preventScroll: true })
})

function send(): void {
  if (empty.value || props.holding)
    return
  emit('send', text.value.trim())
  if (props.holding === undefined)
    text.value = ''
}

function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing)
    return
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    send()
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('cancel')
  }
}

defineExpose({ focus: () => field.value?.focus({ preventScroll: true }) })
</script>

<template>
  <div
    class="flex items-end gap-1 rounded-[18px] py-1 pl-3 pr-1"
    :class="floating ? 'overlay-shell' : 'bg-surface-raised ring-1 ring-line focus-within:ring-line-focus'"
  >
    <textarea
      ref="field"
      v-model="text"
      rows="1"
      :placeholder="placeholder"
      class="max-h-32 min-h-7 flex-1 resize-none bg-transparent py-1 text-chrome leading-5 text-fg outline-none [field-sizing:content] placeholder:text-fg-subtle"
      @keydown="onKeydown"
    />
    <Tooltip :content="sendLabel">
      <IconButton
        :icon="ArrowUp"
        :variant="empty ? 'ghost' : 'accent'"
        circle
        :disabled="empty || holding"
        :aria-label="sendLabel"
        @mousedown.prevent
        @click="send"
      />
    </Tooltip>
  </div>
</template>

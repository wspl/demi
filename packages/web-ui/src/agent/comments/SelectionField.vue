<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { X } from '@lucide/vue'
import { isMacOS } from '@tiptap/core'
import { useFieldScrollbars } from '../../composables/useFieldScrollbars'
import Button from '../../ui/Button.vue'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'
import type { SentenceText } from '../../ui/ui-text'
import { t as shared } from '../../locales/text'
import { t } from '../locales/text'
import type { FieldWay } from './selection-field'

/**
 * The one field a remark on a part of a reply is written in, with its actions
 * under it (`docs/decisions/0054-a-selection-opens-a-comment-field.md`).
 *
 * - **Comment** comments on the selection, **Ask** asks a side question about
 *   it, and **Send** sends the quote and the words to the main agent now. An
 *   action the page does not offer has no button.
 * - Return presses the chosen action, Comment unless a menu's Ask chose it,
 *   and ⌘Return (Control-Return elsewhere) presses Send; Shift-Return starts a
 *   new line.
 * - With no words, Return, Comment and Ask shake the field once, and Send is
 *   dimmed.
 * - ✕ and Escape in the field close it (`close`); its owner also closes it for
 *   a press elsewhere or a new selection. On a phone the ✕ stands at the end
 *   of the sheet's quote (`SelectionFieldQuote.vue`), and the field has none.
 *
 * The owner takes the focus into it (`focus`) and keeps the page's selection
 * for it, so a key typed after the selection, an input method's composition
 * included, starts in the field.
 */
const props = defineProps<{
  phone?: boolean
  /** Whether the page offers Ask; without it the field has no Ask button. */
  ask?: boolean
  /** Whether the page offers Send; without it the field has no Send button. */
  send?: boolean
  /** The action Return presses. */
  way?: FieldWay
  /** Why no action can act now, as the buttons' tooltip, such as a reply still being written. */
  unavailable?: SentenceText | null
  /** An action is under way, as a picture being kept: nothing acts meanwhile. */
  busy?: boolean
}>()

const text = defineModel<string>({ default: '' })

const emit = defineEmits<{
  comment: [text: string]
  ask: [text: string]
  send: [text: string]
  close: []
}>()

const field = ref<HTMLTextAreaElement>()
const frame = ref<HTMLElement>()
const shaking = ref(false)
const empty = computed(() => text.value.trim() === '')
const chosen = computed<FieldWay>(() => props.way ?? 'comment')
const blocked = computed(() => !!props.unavailable || props.busy === true)

// It grows to a few lines and scrolls past them with the app's scrollbar.
useFieldScrollbars(field, frame, () => text.value)

defineExpose({
  /** Takes the focus, the caret at the end of what is typed. */
  focus(): void {
    field.value?.focus({ preventScroll: true })
  },
  /** Whether the focus is in the field. */
  get focused(): boolean {
    return document.activeElement === field.value
  },
})

async function shake(): Promise<void> {
  shaking.value = false
  await nextTick()
  // A frame between, so a second press restarts the animation.
  requestAnimationFrame(() => (shaking.value = true))
}

function act(way: 'comment' | 'ask' | 'send'): void {
  if (blocked.value)
    return
  const words = text.value.trim()
  if (words === '') {
    // Send is dimmed with no words, so it never gets here.
    if (way !== 'send')
      void shake()
    return
  }
  if (way === 'comment')
    emit('comment', words)
  else if (way === 'ask')
    emit('ask', words)
  else
    emit('send', words)
}

function onKeydown(event: KeyboardEvent): void {
  // Safari ends a composition with an Enter whose `isComposing` is already false
  // and whose key code is 229: it confirms the words, and sends nothing.
  if (event.isComposing || event.keyCode === 229)
    return
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault()
    const mod = isMacOS() ? event.metaKey : event.ctrlKey
    if (mod) {
      if (props.send)
        act('send')
    } else
      act(chosen.value === 'ask' && props.ask ? 'ask' : 'comment')
  } else if (event.key === 'Escape') {
    event.preventDefault()
    event.stopPropagation()
    emit('close')
  }
}

/** A press on the actions keeps the focus where it is. */
function keep(event: Event): void {
  event.preventDefault()
}
</script>

<template>
  <div
    data-selection-field
    class="overlay-shell flex max-w-[calc(100vw-1rem)] flex-col gap-1.5 rounded-[14px] p-2"
    :class="[phone ? 'w-full' : 'w-[340px]', { 'selection-field-shake': shaking }]"
    @animationend="shaking = false"
  >
    <div class="flex items-start gap-1">
      <div ref="frame" class="relative flex min-w-0 flex-1">
        <textarea
          ref="field"
          v-model="text"
          data-overlayscrollbars-initialize
          rows="1"
          :placeholder="t('comments.placeholder')"
          :aria-label="t('comments.placeholder')"
          class="max-h-32 min-h-7 min-w-0 flex-1 resize-none bg-transparent px-1 py-1 text-chrome leading-5 text-fg outline-none [field-sizing:content] placeholder:text-fg-subtle"
          @keydown="onKeydown"
        />
      </div>
      <Tooltip v-if="!phone" :content="shared('common.close')">
        <IconButton
          :icon="X"
          size="sm"
          variant="ghost"
          :aria-label="shared('common.close')"
          @mousedown="keep"
          @click="emit('close')"
        />
      </Tooltip>
    </div>
    <div class="flex flex-wrap items-center justify-end gap-1.5">
      <Button
        v-if="send"
        size="sm"
        :disabled="empty || blocked"
        :disabled-reason="empty ? undefined : unavailable ?? undefined"
        @mousedown="keep"
        @click="act('send')"
      >
        {{ t('composer.send') }}
      </Button>
      <Button
        v-if="ask"
        size="sm"
        :variant="chosen === 'ask' ? 'primary' : 'default'"
        :disabled="blocked"
        :disabled-reason="unavailable ?? undefined"
        @mousedown="keep"
        @click="act('ask')"
      >
        {{ t('ask.ask') }}
      </Button>
      <Button
        size="sm"
        :variant="chosen === 'comment' || !ask ? 'primary' : 'default'"
        :disabled="blocked"
        :disabled-reason="unavailable ?? undefined"
        @mousedown="keep"
        @click="act('comment')"
      >
        {{ t('comments.comment') }}
      </Button>
    </div>
  </div>
</template>

<style scoped>
.selection-field-shake {
  animation: selection-field-shake 0.32s ease-out;
}

@keyframes selection-field-shake {
  15%, 55% {
    transform: translateX(-5px);
  }
  35%, 75% {
    transform: translateX(5px);
  }
}

@media (prefers-reduced-motion: reduce) {
  /* Without motion the refusal is a brief dip of the field's opacity. */
  .selection-field-shake {
    animation: selection-field-dip 0.32s ease-out;
  }

  @keyframes selection-field-dip {
    50% {
      opacity: 0.5;
    }
  }
}
</style>

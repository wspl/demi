<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import { Pencil, Reply } from '@lucide/vue'
import { usePhone } from '../../ui/phone'
import Tooltip from '../../ui/Tooltip.vue'
import { LONG_PRESS_MS, LONG_PRESS_SLOP_PX } from '../../ui/context-menu'
import { t } from '../locales/text'

/**
 * The replies the user is likely to send next, as one row of pills under an
 * answer's footer: a click sends a pill's words, its Edit (a long press on a
 * phone) puts them into the composer.
 *
 * The row keeps a line's height while the suggestions are written (`replies`
 * null) and when none came, so nothing under the answer moves as they
 * arrive. Every pill shows its words whole: one that does not fit beside the
 * others goes to the next line, and one wider than the row wraps its words,
 * its ends rounded as a one-line pill's.
 */
const props = defineProps<{
  /** Null while they are written; empty when none came. */
  replies: readonly string[] | null
}>()

const emit = defineEmits<{
  send: [text: string]
  edit: [text: string]
}>()

const phone = usePhone()

/** The press that may become a long press, and whether it did, which then sends nothing. */
let press: { timer: ReturnType<typeof setTimeout>; x: number; y: number } | null = null
let pressedLong = false

function pressDown(event: PointerEvent, text: string): void {
  pressedLong = false
  if (!phone.value || event.pointerType === 'mouse')
    return
  clearPress()
  press = {
    x: event.clientX,
    y: event.clientY,
    timer: setTimeout(() => {
      press = null
      pressedLong = true
      emit('edit', text)
    }, LONG_PRESS_MS),
  }
}

function pressMove(event: PointerEvent): void {
  if (press && Math.hypot(event.clientX - press.x, event.clientY - press.y) > LONG_PRESS_SLOP_PX)
    clearPress()
}

function clearPress(): void {
  if (press)
    clearTimeout(press.timer)
  press = null
}

function send(text: string): void {
  if (pressedLong) {
    pressedLong = false
    return
  }
  emit('send', text)
}

onBeforeUnmount(clearPress)
</script>

<template>
  <div
    data-context-chrome
    role="group"
    :aria-label="t('suggestions.label')"
    class="mt-2 flex min-h-chip flex-wrap items-start gap-2"
  >
    <TransitionGroup name="suggested-reply">
      <div
        v-for="text in props.replies ?? []"
        :key="text"
        class="suggested-reply btn group relative flex min-h-chip max-w-full items-stretch text-chrome text-fg-body"
      >
        <button
          type="button"
          class="suggested-reply flex min-w-0 cursor-default items-start gap-1.5 py-[calc((var(--spacing-chip)-1lh)/2)] pl-2.5 pr-3 text-left"
          @click="send(text)"
          @pointerdown="pressDown($event, text)"
          @pointermove="pressMove"
          @pointerup="clearPress"
          @pointercancel="clearPress"
          @contextmenu="phone && $event.preventDefault()"
        >
          <!-- Centered on the first line. -->
          <span class="flex h-[1lh] shrink-0 items-center">
            <Reply :size="14" aria-hidden="true" class="text-fg-subtle" />
          </span>
          <!-- A word longer than the pill, such as an identifier, breaks too. -->
          <span class="min-w-0 [overflow-wrap:anywhere]">{{ text }}</span>
        </button>
        <!-- Edit stands at the end of the pill's last line, over the words there only. -->
        <Tooltip v-if="!phone" :content="t('suggestions.edit')">
          <button
            type="button"
            :aria-label="t('suggestions.edit')"
            class="suggested-reply suggested-reply-edit absolute bottom-0 right-0 flex h-chip w-10 cursor-default items-center justify-end pr-2.5 text-fg-subtle opacity-0 hover:text-fg focus-visible:opacity-100 group-hover:opacity-100"
            @click.stop="emit('edit', text)"
          >
            <Pencil :size="14" />
          </button>
        </Tooltip>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
/* A one-line pill's round ends, which a pill of more lines keeps at its corners. */
.suggested-reply {
  border-radius: calc(var(--spacing-chip) / 2);
}
/* In light the pill is as white as the reading surface under it, where a button's
   half-pixel hairline all but vanishes, on a phone most: its edge is a whole pixel. */
html[data-theme="light"] .suggested-reply.btn {
  --shadow-btn: 0 0 0 1px rgb(0 0 0 / 0.12);
  --shadow-btn-hover: 0 0 0 1px rgb(0 0 0 / 0.18);
}
/* The Edit button lies over the pill's end, on the pill's own fill, the words fading under it. */
.suggested-reply-edit {
  background: linear-gradient(to right, transparent, var(--fill-color) 55%);
}
.suggested-reply-enter-active {
  transition: opacity 150ms ease-out;
}
.suggested-reply-enter-from {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .suggested-reply-enter-active {
    transition: none;
  }
}
</style>

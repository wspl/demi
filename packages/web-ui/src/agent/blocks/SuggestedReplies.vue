<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import { Lightbulb, Pencil, Reply } from '@lucide/vue'
import { usePhone } from '../../ui/phone'
import Tooltip from '../../ui/Tooltip.vue'
import { isTextCut } from '../../ui/truncation'
import { LONG_PRESS_MS, LONG_PRESS_SLOP_PX } from '../../ui/context-menu'
import { t } from '../locales/text'
import type { SuggestedReply } from '../suggested-replies'

/**
 * The replies the user is likely to send next, as one row of pills under an
 * answer's footer: a click sends a pill's words, its Edit (a long press on a
 * phone) puts them into the composer.
 *
 * The row keeps its height while the suggestions are written (`replies` null)
 * and when none came, so nothing under the answer moves as they arrive. The
 * pills stay on one line: off a phone they narrow and end in an ellipsis,
 * the tooltip showing the words whole; on a phone each shows its words
 * whole, as a tap sends them, and the line scrolls sideways.
 */
const props = defineProps<{
  /** Null while they are written; empty when none came. */
  replies: readonly SuggestedReply[] | null
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
    class="suggested-replies mt-2 flex h-chip items-center gap-2 phone:-mx-[var(--agent-pad-x,2rem)] phone:overflow-x-auto phone:px-[var(--agent-pad-x,2rem)]"
  >
    <TransitionGroup name="suggested-reply">
      <div
        v-for="{ text, kind } in props.replies ?? []"
        :key="text"
        class="btn group relative flex h-chip min-w-0 max-w-full shrink items-center rounded-full text-chrome text-fg-body phone:max-w-none phone:shrink-0"
      >
        <button
          type="button"
          class="flex h-full min-w-0 cursor-default items-center gap-1.5 rounded-full pl-2.5 pr-3 text-left"
          @click="send(text)"
          @pointerdown="pressDown($event, text)"
          @pointermove="pressMove"
          @pointerup="clearPress"
          @pointercancel="clearPress"
          @contextmenu="phone && $event.preventDefault()"
        >
          <!-- An answer to the reply, or a new topic. -->
          <component :is="kind === 'answer' ? Reply : Lightbulb" :size="14" aria-hidden="true" class="shrink-0 text-fg-subtle" />
          <!-- A cut pill shows its words whole in its tip, as a cut title does. -->
          <Tooltip tag="span" :content="text" :show-if="isTextCut" :disabled="phone" class="min-w-0 truncate phone:overflow-visible phone:whitespace-nowrap">
            {{ text }}
          </Tooltip>
        </button>
        <Tooltip v-if="!phone" :content="t('suggestions.edit')">
          <button
            type="button"
            :aria-label="t('suggestions.edit')"
            class="suggested-reply-edit absolute inset-y-0 right-0 flex w-10 cursor-default items-center justify-end pr-2.5 rounded-r-full text-fg-subtle opacity-0 hover:text-fg focus-visible:opacity-100 group-hover:opacity-100"
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
.suggested-replies {
  scrollbar-width: none;
}
.suggested-replies::-webkit-scrollbar {
  display: none;
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

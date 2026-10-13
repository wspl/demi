<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useElementSize, usePreferredReducedMotion } from '@vueuse/core'
import { useFileDrop } from '../composables/useFileDrop'
import DropOutline from '../ui/DropOutline.vue'
import { ROLLED_UP_CLIP, SHOWN_CLIP, drawnTop, glideFrom, unroll } from '../ui/glide'
import { dataTransferFiles } from './message-input/attachments'
import { transferCopiedMessage } from './message-input/copied-message'

/**
 * The composer's frame. Files dragged over it can be dropped anywhere on it:
 * a dashed line runs inside its edge, so the target is the whole composer
 * and not one control, and the editor marks where in the text they will land.
 *
 * The box holds the attach control, the message and the actions; the `below`
 * slot is a row under it, on the right, for what belongs to the conversation
 * and not to the message (the model, the context), aligned to the box's right
 * edge. Its `below-start` slot is the same row's left end, for what lists
 * the conversation's messages saved for later; the `list` slot is a panel under that row.
 *
 * The list opens and closes in 150 ms: the layout takes its new shape at
 * once, so the transcript above resizes once, and the frame glides from where
 * it was drawn to its new place while the list unrolls beneath it, or rolls
 * up as it leaves, through transforms and clips alone (`product.md` § Saved
 * for later). A toggle in the midst turns the glide around from where it is
 * drawn; reduced motion shows the change at once.
 *
 * The message's line comes first. The editor slot hears what the line offers
 * its text (`line`), so a message that outgrows it opens the shell instead of
 * running off its end.
 *
 * A ruler measures the line in both shapes: on one line it lies in the
 * editor's own cell, and in the open shell it takes the editor's column in
 * the row of controls below, which is the same width. So the editor hears one
 * width that does not change with the shape, and the controls neither jump
 * nor drive each other when the shell opens and closes.
 */

const props = defineProps<{
  focused?: boolean
  expanded?: boolean
  dropping?: boolean
}>()

const emit = defineEmits<{
  /** Files dropped, with the drop that places them. */
  dropFiles: [files: File[], event: DragEvent]
}>()

const ruler = ref<HTMLElement | null>(null)
const { width: line } = useElementSize(ruler)

const shell = ref<HTMLElement | null>(null)
const { over: dragOver } = useFileDrop(shell, {
  drop(event) {
    const transfer = event.dataTransfer
    // A message dragged out of another one brings its files without their
    // bytes: the editor takes it (`intake.ts`), not its files.
    if (!transfer || transferCopiedMessage(transfer))
      return
    const files = dataTransferFiles(transfer)
    if (files.length > 0)
      emit('dropFiles', files, event)
  },
})
const showDrop = computed(() => props.dropping === true || dragOver.value)

/** How long the list takes to open or close. */
const LIST_GLIDE_MS = 150
const GLIDE: KeyframeAnimationOptions = { duration: LIST_GLIDE_MS, easing: 'cubic-bezier(0.2, 0, 0, 1)' }
const motion = usePreferredReducedMotion()
const frame = ref<HTMLElement | null>(null)
/** The glide under way: the frame's and the list's, which a toggle or the unmount cancels. */
let gliding: Animation[] = []
/** Where the frame was drawn, and the list's clip mid-glide, as the layout began to change. */
let before: { top: number; clip: string } | null = null

/** Reads where the frame and the list are drawn, then stops a glide under way there. */
function measure(list: Element): void {
  before = frame.value ? { top: drawnTop(frame.value), clip: getComputedStyle(list).clipPath } : null
  for (const animation of gliding.splice(0)) {
    animation.cancel()
  }
}

/** Plays the glide from what `measure` read to the layout now applied; `done` once it ends. */
function glide(list: Element, to: string, done: () => void): void {
  const start = before
  before = null
  if (!start || !frame.value || motion.value === 'reduce') {
    done()
    return
  }
  // A list that was not drawn yet, or drawn whole, starts from the other end.
  const from = start.clip === '' || start.clip === 'none' ? (to === SHOWN_CLIP ? ROLLED_UP_CLIP : SHOWN_CLIP) : start.clip
  const moves = [glideFrom(frame.value, start.top, GLIDE), unroll(list, from, to, GLIDE)]
  gliding = moves.filter((move): move is Animation => move !== null)
  // A cancelled glide rejects; whoever cancelled it plays its own, or the page is gone.
  Promise.all(gliding.map((move) => move.finished)).then(done, () => {})
}

/** The list leaves the layout at once, kept where it stood until it has rolled up. */
function leave(element: Element, done: () => void): void {
  const list = element as HTMLElement
  list.style.position = 'absolute'
  list.style.insetInline = '0'
  list.style.top = `${list.offsetTop}px`
  glide(list, ROLLED_UP_CLIP, done)
}

onBeforeUnmount(() => {
  for (const animation of gliding.splice(0)) {
    animation.cancel()
  }
})
</script>

<template>
  <div ref="frame" class="composer-frame relative">
    <div
      ref="shell"
      class="input-float composer-shell relative bg-surface-raised outline outline-1 transition-[outline-color] duration-200"
      :class="[
        showDrop ? 'outline-transparent' : focused ? 'outline-line-focus' : 'outline-line',
        expanded ? 'composer-shell-expanded' : 'composer-shell-capsule',
      ]"
      :data-dropping="showDrop ? '' : undefined"
    >
      <div class="composer-attach">
        <slot name="attach" />
      </div>
      <div class="composer-editor">
        <slot name="editor" :line="line" />
      </div>
      <div
        ref="ruler"
        class="composer-line-ruler"
        aria-hidden="true"
      />
      <div class="composer-actions flex items-center gap-1">
        <slot name="actions" />
      </div>
      <DropOutline
        v-if="showDrop"
        class="z-20"
        radius="var(--composer-shell-radius)"
      />
    </div>
    <div class="composer-below">
      <div v-if="$slots['below-start']" class="composer-below-start">
        <slot name="below-start" />
      </div>
      <slot name="below" />
    </div>
    <Transition
      :css="false"
      @before-enter="measure"
      @enter="(element, done) => glide(element, SHOWN_CLIP, done)"
      @before-leave="measure"
      @leave="leave"
    >
      <div v-if="$slots['list']" class="composer-list">
        <slot name="list" />
      </div>
    </Transition>
  </div>
</template>

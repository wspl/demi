<script setup lang="ts">
import { computed, onBeforeUnmount, ref, useSlots } from 'vue'
import Popover from './Popover.vue'
import { useLayerElevation } from '../overlay/layerElevation'
import { appOverlayStore } from '../overlay/appOverlay'
import type { OverlayStore } from '../overlay/overlayStore'

/**
 * A card the pointer or focus opens over a trigger, which the user can reach
 * and act in: the pointer crosses into it and it stays, a click on the
 * trigger does nothing by itself, and its controls may carry their own tips.
 * It closes when the pointer and the focus have left both, on Escape and on
 * a click outside. Like a tip, it does not open over another menu, popover
 * or dialog.
 *
 * The card is one row: what it says, and an optional `action` at its end,
 * one `sm` Button, which keeps the same inset from the card's top, end and
 * bottom. Optional `details` follow below a divider, such as a list whose
 * entries act; they get `close`, for an entry whose action leaves the card.
 */
const props = withDefaults(defineProps<{
  /** Whether the card opens above the trigger or below it; it grows toward the inside of the trigger's region (Popover). */
  side?: 'top' | 'bottom'
  openDelayMs?: number
  /** Time the pointer has to cross from the trigger into the card. */
  closeDelayMs?: number
  overlayStore?: OverlayStore
  /** Shown whatever the pointer does, as a gallery specimen pins it inside an overlay well. */
  pinned?: boolean
}>(), {
  side: 'top',
  openDelayMs: 120,
  closeDelayMs: 150,
})

const slots = useSlots()
const triggerRef = ref<HTMLElement | null>(null)
const cardRef = ref<HTMLElement | null>(null)
const elevation = useLayerElevation()
/** Opened by the pointer or the focus. */
const shown = ref(false)
const isOpen = computed(() => props.pinned || shown.value)
let openTimer: ReturnType<typeof setTimeout> | null = null
let closeTimer: ReturnType<typeof setTimeout> | null = null

function overlayStore(): OverlayStore {
  return props.overlayStore ?? appOverlayStore
}

function clearTimers(): void {
  if (openTimer) {
    clearTimeout(openTimer)
    openTimer = null
  }
  if (closeTimer) {
    clearTimeout(closeTimer)
    closeTimer = null
  }
}

function open(): void {
  if (shown.value || overlayStore().hasExclusive()) {
    return
  }
  shown.value = true
}

function close(): void {
  clearTimers()
  shown.value = false
}

function scheduleOpen(): void {
  clearTimers()
  openTimer = setTimeout(() => {
    openTimer = null
    open()
  }, props.openDelayMs)
}

function scheduleClose(): void {
  clearTimers()
  closeTimer = setTimeout(close, props.closeDelayMs)
}

/** Focus that moves between the trigger and the card keeps it open. */
function onFocusOut(event: FocusEvent): void {
  const next = event.relatedTarget
  if (next instanceof Node && (triggerRef.value?.contains(next) || cardRef.value?.contains(next))) {
    return
  }
  scheduleClose()
}

onBeforeUnmount(clearTimers)
</script>

<template>
  <span
    ref="triggerRef"
    class="inline-flex"
    @mouseenter="scheduleOpen"
    @mouseleave="scheduleClose"
    @focusin="scheduleOpen"
    @focusout="onFocusOut"
  >
    <slot />
  </span>
  <Popover
    :is-open="isOpen"
    :overlay-store="overlayStore()"
    :anchor-el="triggerRef"
    :side="side"
    :offset="8"
    :ignore-els="triggerRef ? [triggerRef] : []"
    @close="close"
  >
    <div
      ref="cardRef"
      class="overlay-shell rounded-md text-xs text-fg"
      :style="{ '--elevation': elevation }"
      @mouseenter="clearTimers"
      @mouseleave="scheduleClose"
      @focusout="onFocusOut"
    >
      <div class="hover-card">
        <div class="hover-card-body min-w-0 flex-1">
          <slot name="card" />
        </div>
        <div v-if="slots.action" class="hover-card-action">
          <slot name="action" />
        </div>
      </div>
      <div v-if="slots.details" class="border-t border-line">
        <slot name="details" :close="close" />
      </div>
    </div>
  </Popover>
</template>

<style scoped>
/* The card's height and side padding are named so the action cell derives
   its inset from them: (card height - control height) / 2 on every edge it
   touches. The control is an `sm` Button, 1.5rem high. */
.hover-card {
  --hover-card-h: 2.25rem;
  --hover-card-px: 0.75rem;
  --hover-card-action-h: 1.5rem;
  display: flex;
  align-items: stretch;
  min-height: var(--hover-card-h);
  padding-left: var(--hover-card-px);
}

.hover-card-body {
  display: flex;
  align-items: center;
  padding-block: 0.5rem;
  padding-right: var(--hover-card-px);
  line-height: 1.25rem;
}

.hover-card-action {
  display: flex;
  align-items: center;
  padding-right: calc((var(--hover-card-h) - var(--hover-card-action-h)) / 2);
}
</style>

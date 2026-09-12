<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import {
  clampSize,
  sizeFromDrag,
  sizeFromKey,
  type ResizeOrientation,
  type ResizeSide,
} from './resize-handle'

/**
 * The divider between two panes that sizes one of them. It owns nothing but
 * the gesture: the host keeps the size (v-model) and lays the panes out.
 *
 * Pointer: the handle captures the pointer on press, so the drag survives
 * leaving the handle, the window and iframes; while it lasts the document
 * wears `is-resizing-x`/`-y`, which fixes the cursor and stops text selection
 * everywhere. Moves are coalesced to one update per frame, and the last one
 * is flushed on release so the pane ends exactly under the pointer. Release,
 * cancel, lost capture and the window losing focus all end the drag the same
 * way; Escape ends it by putting the size back where it started.
 *
 * Keyboard: the handle is a focusable separator. The arrows along its axis
 * move the edge by `step` (four steps with Shift), Home and End go to the
 * bounds. A double-click returns to `defaultValue` when the host gives one.
 *
 * The hit area is 8px wide and straddles the boundary through negative
 * margins, so it takes no room in the layout; the line inside it shows on
 * hover, focus and while dragging.
 */
const props = withDefaults(
  defineProps<{
    modelValue: number
    min: number
    max: number
    /** The size a double-click returns to; no double-click without it. */
    defaultValue?: number
    orientation?: ResizeOrientation
    /** Which side of the handle the sized pane is on. */
    side?: ResizeSide
    /** One arrow press, in px. */
    step?: number
    /** The separator's accessible name: what it resizes. */
    label: string
    disabled?: boolean
  }>(),
  {
    orientation: 'vertical',
    side: 'start',
    step: 16,
    disabled: false,
  },
)

const emit = defineEmits<{
  'update:modelValue': [value: number]
  /** The size settled: a drag ended, a key or double-click applied. Persist here. */
  commit: [value: number]
}>()

const vertical = computed(() => props.orientation === 'vertical')
const bounds = computed(() => ({ min: props.min, max: props.max }))
const dragging = ref(false)
const focused = ref(false)
// Focus taken by a press shows no line; focus reached or used by the keyboard does.
const focusByPointer = ref(false)
const root = ref<HTMLElement | null>(null)

interface Drag {
  pointerId: number
  startPointer: number
  startValue: number
  /** The size the next frame will emit, when it differs from the last one emitted. */
  pending: number | null
  frame: number | null
  emitted: number
}
let drag: Drag | null = null

function documentClass(): string {
  return vertical.value ? 'is-resizing-x' : 'is-resizing-y'
}

function update(value: number): void {
  if (value !== props.modelValue) {
    emit('update:modelValue', value)
  }
}

function pointerPosition(event: PointerEvent): number {
  return vertical.value ? event.clientX : event.clientY
}

function onPointerDown(event: PointerEvent): void {
  if (props.disabled || event.button !== 0 || drag) {
    return
  }
  // No text selection starts under the press; the handle takes focus itself so
  // the arrows work right after a drag, without the keyboard focus line.
  event.preventDefault()
  const target = event.currentTarget as HTMLElement
  focusByPointer.value = true
  target.focus({ preventScroll: true })
  target.setPointerCapture(event.pointerId)
  drag = {
    pointerId: event.pointerId,
    startPointer: pointerPosition(event),
    startValue: clampSize(props.modelValue, bounds.value),
    pending: null,
    frame: null,
    emitted: props.modelValue,
  }
  dragging.value = true
  document.documentElement.classList.add(documentClass())
  window.addEventListener('blur', onWindowBlur)
  window.addEventListener('keydown', onWindowKeydown, true)
}

function onPointerMove(event: PointerEvent): void {
  if (!drag || event.pointerId !== drag.pointerId) {
    return
  }
  const next = sizeFromDrag(
    drag.startValue,
    pointerPosition(event) - drag.startPointer,
    props.side,
    bounds.value,
  )
  if (next === drag.emitted) {
    drag.pending = null
    return
  }
  drag.pending = next
  if (drag.frame === null) {
    drag.frame = requestAnimationFrame(flush)
  }
}

function flush(): void {
  if (!drag) {
    return
  }
  drag.frame = null
  if (drag.pending !== null) {
    drag.emitted = drag.pending
    drag.pending = null
    update(drag.emitted)
  }
}

function onPointerEnd(event: PointerEvent): void {
  if (drag && event.pointerId === drag.pointerId) {
    finish(drag.pending ?? drag.emitted)
  }
}

function onWindowBlur(): void {
  if (drag) {
    finish(drag.pending ?? drag.emitted)
  }
}

function onWindowKeydown(event: KeyboardEvent): void {
  if (drag && event.key === 'Escape') {
    event.preventDefault()
    finish(drag.startValue)
  }
}

/** Ends the drag at `value`: the frame is dropped, the value applied, the document restored. */
function finish(value: number): void {
  const current = drag
  if (!current) {
    return
  }
  drag = null
  if (current.frame !== null) {
    cancelAnimationFrame(current.frame)
  }
  dragging.value = false
  document.documentElement.classList.remove(documentClass())
  window.removeEventListener('blur', onWindowBlur)
  window.removeEventListener('keydown', onWindowKeydown, true)
  const element = root.value
  if (element?.hasPointerCapture(current.pointerId)) {
    element.releasePointerCapture(current.pointerId)
  }
  update(value)
  emit('commit', value)
}

function onKeydown(event: KeyboardEvent): void {
  if (props.disabled || drag) {
    return
  }
  const next = sizeFromKey(event.key, event.shiftKey, props.modelValue, {
    ...bounds.value,
    orientation: props.orientation,
    side: props.side,
    step: props.step,
  })
  if (next === null) {
    return
  }
  event.preventDefault()
  focusByPointer.value = false
  update(next)
  emit('commit', next)
}

function onBlur(): void {
  focused.value = false
  focusByPointer.value = false
}

function onDoubleClick(): void {
  if (props.disabled || props.defaultValue === undefined) {
    return
  }
  const next = clampSize(props.defaultValue, bounds.value)
  update(next)
  emit('commit', next)
}

onBeforeUnmount(() => {
  if (drag) {
    finish(drag.emitted)
  }
})
</script>

<template>
  <div
    ref="root"
    role="separator"
    :aria-orientation="orientation"
    :aria-valuemin="min"
    :aria-valuemax="max"
    :aria-valuenow="clampSize(modelValue, bounds)"
    :aria-label="label"
    :aria-disabled="disabled || undefined"
    :tabindex="disabled ? -1 : 0"
    class="group/handle relative z-10 shrink-0 select-none touch-none outline-none"
    :class="[
      vertical ? 'w-2 -mx-1' : 'h-2 -my-1',
      disabled ? '' : vertical ? 'cursor-col-resize' : 'cursor-row-resize',
    ]"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerEnd"
    @pointercancel="onPointerEnd"
    @lostpointercapture="onPointerEnd"
    @keydown="onKeydown"
    @dblclick="onDoubleClick"
    @focus="focused = true"
    @blur="onBlur"
  >
    <!-- The line: hidden until the pointer rests on the handle, lit while it drags or has focus. -->
    <span
      aria-hidden="true"
      class="absolute rounded-full transition-colors duration-150 group-hover/handle:delay-150"
      :class="[
        vertical ? 'inset-y-0 left-1/2 w-0.5 -translate-x-1/2' : 'inset-x-0 top-1/2 h-0.5 -translate-y-1/2',
        dragging || (focused && !focusByPointer)
          ? 'bg-accent-fill delay-0'
          : disabled
            ? ''
            : 'group-hover/handle:bg-line-strong',
      ]"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import {
  clampSize,
  sizeFromDrag,
  sizeFromKey,
  type ResizeOrientation,
  type ResizeSide,
} from './resize-handle'
import { startPointerDrag } from './pointer-drag'

/**
 * The divider between two panes that sizes one of them. It owns nothing but
 * the gesture: the host keeps the size (v-model) and lays the panes out.
 *
 * Pointer: a press starts a pointer drag (`startPointerDrag`), which keeps
 * the resize cursor on the whole page and reports one size a frame; Escape
 * ends it by putting the size back where it started.
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

/** The drag under way: the last size it reached, and what ends it early. */
let drag: { value: number; stop: () => void } | null = null

function update(value: number): void {
  if (value !== props.modelValue) {
    emit('update:modelValue', value)
  }
}

function onPointerDown(event: PointerEvent): void {
  if (props.disabled || event.button !== 0 || drag) {
    return
  }
  // No text selection starts under the press; the handle takes focus itself so
  // the arrows work right after a drag, without the keyboard focus line.
  event.preventDefault()
  focusByPointer.value = true
  root.value?.focus({ preventScroll: true })
  const startValue = clampSize(props.modelValue, bounds.value)
  dragging.value = true
  const stop = startPointerDrag(event, {
    cursor: vertical.value ? 'col-resize' : 'row-resize',
    move(dx, dy) {
      if (!drag)
        return
      drag.value = sizeFromDrag(startValue, vertical.value ? dx : dy, props.side, bounds.value)
      update(drag.value)
    },
    end(cancelled) {
      const value = cancelled ? startValue : drag?.value ?? startValue
      drag = null
      dragging.value = false
      update(value)
      emit('commit', value)
    },
  })
  drag = { value: startValue, stop }
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
  drag?.stop()
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

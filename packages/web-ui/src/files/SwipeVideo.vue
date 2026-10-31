<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import MediaPreview from './MediaPreview.vue'
import { swipeAxis, type StepReach, type SwipeAxis } from './swipe-step'
import { useSwipeLayer } from './swipe-layer'

/**
 * A video the media viewer shows, in the web browser's own player, which a
 * finger swipes as a picture is swiped (`swipe-layer.ts`):
 *
 * - Sideways across the video, with `steps`, it steps to the neighbour.
 * - Down across the video, where it is `dismissible`, it pulls down, fading,
 *   and closes once let go far enough down.
 * - A tap on the video is the player's, as is anything on its controls along
 *   the bottom, such as dragging the playhead: only a finger that moves on the
 *   video's picture becomes a swipe.
 * - Where it is `dismissible`, a tap beside the video asks to close it.
 *
 * A mouse never swipes it.
 */
const props = defineProps<{
  src: string
  name: string
  /** It is out of sight, and never plays meanwhile (`MediaPreview`). */
  hidden: boolean
  /** Edge to edge, with no margin around the player. */
  flush: boolean
  /** The neighbours a swipe can step to; none when the video is not swiped sideways. */
  steps?: StepReach
  /** The side a swipe brought this video from. */
  from?: -1 | 0 | 1
  /** A tap beside the video or a pull down closes it, where it covers the screen. */
  dismissible?: boolean
}>()
const emit = defineEmits<{
  size: [width: number, height: number]
  failed: []
  step: [direction: -1 | 1, slid: boolean]
  dismiss: []
}>()

/**
 * How far up from the video's bottom edge the player's controls reach on a
 * phone, the playhead's track among them: a press there stays the player's.
 */
const CONTROLS_PX = 56

const frame = ref<HTMLElement | null>(null)
const slider = ref<HTMLElement | null>(null)
const { width, height } = useElementSize(frame)
const swipe = useSwipeLayer(slider, () => (width.value > 0 && height.value > 0 ? { width: width.value, height: height.value } : null), {
  reach: () => props.steps,
  dismissible: () => props.dismissible ?? false,
  step: (direction, slid) => emit('step', direction, slid),
  dismiss: () => emit('dismiss'),
})

/** Ends the press under way as a lift the finger did not make. */
let stopPress: (() => void) | null = null
/** Another finger came down during the press under way, which is then no tap. */
let spoiled = false

function within(event: PointerEvent, box: DOMRect): boolean {
  return event.clientX >= box.left && event.clientX <= box.right && event.clientY >= box.top && event.clientY <= box.bottom
}

/**
 * A press on the stage: on the video's picture it waits to see whether the
 * finger moves, leaving a tap to the player; beside the video it is the
 * stage's alone, so nothing around takes it for its own (`MediaViewer`).
 */
function onPress(event: PointerEvent): void {
  if (event.button !== 0)
    return
  const box = frame.value?.querySelector('video')?.getBoundingClientRect()
  const onVideo = box !== undefined && within(event, box)
  if (onVideo && event.clientY > box.bottom - CONTROLS_PX)
    return
  if (!onVideo)
    event.preventDefault()
  // A second finger makes the press under way no tap, as a pinch is none.
  if (stopPress) {
    spoiled = true
    return
  }
  if (swipe.leaving())
    return
  spoiled = false
  swipe.hold()
  const { pointerId } = event
  const startX = event.clientX
  const startY = event.clientY
  const pressedAt = performance.now()
  const swipes = event.pointerType !== 'mouse'
  let axis: SwipeAxis = 'click'
  let claimed = false
  let last = { dx: 0, dy: 0 }

  function move(next: PointerEvent): void {
    if (next.pointerId !== pointerId)
      return
    const dx = next.clientX - startX
    const dy = next.clientY - startY
    if (axis === 'click')
      axis = swipeAxis(dx, dy)
    const takes = swipes && ((axis === 'horizontal' && props.steps !== undefined) || (axis === 'vertical' && props.dismissible))
    if (!takes)
      return
    if (!claimed) {
      claimed = true
      frame.value?.setPointerCapture(pointerId)
      frame.value?.addEventListener('lostpointercapture', lost)
    }
    last = { dx, dy }
    swipe.follow(axis, dx, dy)
  }

  // Release, cancel, a lost capture and the window losing focus end the press, as a drag's end does
  // (`pointer-drag.ts`); only the first is the finger's own lift.
  function finish(released: boolean): void {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', end)
    window.removeEventListener('pointercancel', end)
    window.removeEventListener('blur', blurred)
    frame.value?.removeEventListener('lostpointercapture', lost)
    stopPress = null
    if (claimed)
      swipe.release(axis, last.dx, last.dy, released && !spoiled, performance.now() - pressedAt)
    else if (released && !spoiled && axis === 'click' && !onVideo && props.dismissible)
      emit('dismiss')
  }

  function end(lifted: PointerEvent): void {
    if (lifted.pointerId === pointerId)
      finish(lifted.type === 'pointerup')
  }

  // Only the stage's own loss: the video's implicit capture, which the stage takes over, ends in one
  // that bubbles up to it.
  function lost(event: PointerEvent): void {
    if (event.pointerId === pointerId && event.target === frame.value)
      finish(false)
  }

  function blurred(): void {
    finish(false)
  }

  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', end)
  window.addEventListener('pointercancel', end)
  window.addEventListener('blur', blurred)
  stopPress = () => finish(false)
}

onMounted(() => {
  // A video that a swipe brought slides in from the side it came from.
  if (props.from)
    swipe.arrive(props.from)
})

onBeforeUnmount(() => {
  stopPress?.()
  swipe.dispose()
})
</script>

<template>
  <div ref="frame" class="relative h-full w-full touch-none overflow-hidden" @pointerdown="onPress">
    <div ref="slider" class="absolute inset-0" :style="swipe.style.value">
      <MediaPreview
        :src="src"
        kind="video"
        :name="name"
        autoplay
        :hidden="hidden"
        :flush="flush"
        @size="(w, h) => emit('size', w, h)"
        @failed="emit('failed')"
      />
    </div>
  </div>
</template>

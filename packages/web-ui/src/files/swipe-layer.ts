import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { useMediaQuery } from '@vueuse/core'
import { pullDrops, pullOpacity, swipeOffset, swipeStep, type StepReach, type SwipeAxis } from './swipe-step'
import type { PixelSize } from './thumbnail'

/**
 * The layer a medium the media viewer shows slides on while a finger swipes
 * it, a picture's (`ImagePreview`) or a video's (`MediumStage`), and what the
 * swipe ends in (`swipe-step.ts`):
 *
 * - Sideways, the layer follows the finger, then slides out to the next or
 *   the previous medium, or springs back.
 * - Down, where the medium may close, the layer follows the finger and fades,
 *   then closes the medium or springs back. Only a finger's own lift closes
 *   it: a swipe the system cancels, or one cut short, springs back.
 * - A medium a swipe brought slides in from the side it came from.
 * - With reduced motion nothing slides: the swipe only steps or closes.
 *
 * Its owner decides which presses are swipes and reports their moves and end.
 */
export interface SwipeLayer {
  /** The layer's look while a swipe moves it, or nothing at rest. */
  style: ComputedRef<{ transform: string; opacity?: string } | undefined>
  /** A slide out is under way: a press waits for the medium that comes next. */
  leaving(): boolean
  /** A press takes the layer where the finger finds it, stopping a slide under way. */
  hold(): void
  /** The finger has moved (`dx`, `dy`) from the press along `axis`. */
  follow(axis: SwipeAxis, dx: number, dy: number): void
  /**
   * The swipe ended (`dx`, `dy`) from the press along `axis`, `elapsedMs`
   * after it: `released` when the finger itself lifted.
   */
  release(axis: SwipeAxis, dx: number, dy: number, released: boolean, elapsedMs: number): void
  /** Another medium shows: the layer rests in the middle. */
  reset(): void
  /** The medium that arrived from the side `from`, 1 the right and -1 the left, slides in. */
  arrive(from: -1 | 0 | 1): void
  /** The medium goes: a slide under way stops. */
  dispose(): void
}

/** What the layer slides and asks of its owner. */
export interface SwipeLayerOptions {
  /** The neighbours a swipe can step to; none when the medium is not swiped sideways. */
  reach(): StepReach | undefined
  /** Whether a pull down closes the medium. */
  dismissible(): boolean
  /** A swipe stepped to the next (1) or the previous (-1) medium; `slid` when this one slid out. */
  step(direction: -1 | 1, slid: boolean): void
  /** A pull down closes the medium. */
  dismiss(): void
}

export function useSwipeLayer(layer: Ref<HTMLElement | null>, room: () => PixelSize | null, options: SwipeLayerOptions): SwipeLayer {
  const reduceMotion = useMediaQuery('(prefers-reduced-motion: reduce)')
  const across = ref(0)
  const down = ref(0)
  /** The slide or spring back under way, if any. */
  let sliding: Animation | null = null
  let leaving = false

  const style = computed(() => {
    if (!across.value && !down.value)
      return undefined
    const opacity = down.value ? pullOpacity(down.value, room()?.height ?? 0) : 1
    return { transform: `translate(${across.value}px, ${down.value}px)`, opacity: opacity < 1 ? String(opacity) : undefined }
  })

  /** Slides the layer across from `from` to `to` px and rests it there; with reduced motion it only goes. */
  async function slide(from: number, to: number): Promise<void> {
    const element = layer.value
    sliding?.cancel()
    sliding = null
    if (!element || reduceMotion.value) {
      across.value = to
      return
    }
    const animation = element.animate(
      [{ transform: `translateX(${from}px)` }, { transform: `translateX(${to}px)` }],
      { duration: 180, easing: 'ease-out', fill: 'forwards' },
    )
    sliding = animation
    try {
      await animation.finished
    }
    catch {
      // Cancelled by a newer slide or by the medium going: that one rests the layer.
      return
    }
    across.value = to
    animation.cancel()
    sliding = null
  }

  /** A pull down let go short of closing, or cut short: the layer eases back up, opaque again. */
  function springBack(): void {
    const element = layer.value
    const from = down.value
    down.value = 0
    if (!element || reduceMotion.value || from === 0)
      return
    sliding?.cancel()
    sliding = element.animate(
      [{ transform: `translateY(${from}px)`, opacity: pullOpacity(from, room()?.height ?? 0) }, { transform: 'translateY(0)', opacity: 1 }],
      { duration: 180, easing: 'ease-out' },
    )
  }

  /** A sideways swipe ended: `step` is the medium it goes to, or 0 to spring back. */
  async function settle(step: -1 | 0 | 1): Promise<void> {
    const width = room()?.width ?? 0
    if (step === 0) {
      await slide(across.value, 0)
      return
    }
    if (reduceMotion.value) {
      options.step(step, false)
      return
    }
    leaving = true
    await slide(across.value, -step * width)
    options.step(step, true)
  }

  return {
    style,
    leaving: () => leaving,
    hold() {
      sliding?.cancel()
      sliding = null
    },
    follow(axis, dx, dy) {
      if (reduceMotion.value)
        return
      const reach = options.reach()
      if (axis === 'horizontal' && reach)
        across.value = swipeOffset(dx, reach)
      else if (axis === 'vertical' && options.dismissible())
        down.value = Math.max(0, dy)
    },
    release(axis, dx, dy, released, elapsedMs) {
      const reach = options.reach()
      const now = room()
      if (axis === 'horizontal' && reach) {
        void settle(released && now ? swipeStep(dx, now.width, elapsedMs, reach) : 0)
      }
      else if (axis === 'vertical') {
        if (released && options.dismissible() && pullDrops(dy))
          options.dismiss()
        else
          springBack()
      }
    },
    reset() {
      down.value = 0
    },
    arrive(from) {
      // At its mount the layer knows its width before its owner has measured the room.
      const width = layer.value?.clientWidth ?? room()?.width
      if (from && width)
        void slide(from * width, 0)
    },
    dispose() {
      sliding?.cancel()
    },
  }
}

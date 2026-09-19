import { onBeforeUnmount, ref, watch } from 'vue'

export interface ButtonIconSpinProps {
  spinning?: boolean
  spinOnClick?: boolean
  disabled?: boolean
}

/**
 * Turns a button's icons a whole revolution at a time: one per click with
 * `spinOnClick`, and on and on while `spinning`. A revolution always
 * finishes, so work that ends at once still shows a full turn. `icons` names
 * the elements that turn; the button knows which of its elements they are.
 */
export function useButtonIconSpin(
  icons: () => readonly Element[],
  props: ButtonIconSpinProps,
  onEnd: () => void
) {
  const rotating = ref(false)
  let animations: Animation[] = []
  let disposed = false

  async function start() {
    if (rotating.value || disposed)
      return
    const targets = icons()
    if (!targets.length)
      return
    rotating.value = true
    try {
      do {
        const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
        animations = targets.map((icon) => icon.animate(
          [
            { transform: 'rotate(0deg)' },
            {
              transform: `rotate(${reduceMotion ? 0 : 360}deg)`
            }
          ],
          { duration: 600, easing: 'linear' },
        ))
        await Promise.all(animations.map((animation) => animation.finished))
      } while (props.spinning && !disposed)
      rotating.value = false
      onEnd()
    } catch {
      // Unmount cancels animations and their finished promises.
    }
  }

  function startWhileSpinning() {
    if (props.spinning) {
      void start()
    }
  }

  function onClick() {
    if (props.spinOnClick && !props.disabled) {
      void start()
    }
  }

  watch(
    () => props.spinning,
    startWhileSpinning,
    { flush: 'post', immediate: true }
  )
  // A button that mounts while `spinning`, or whose icon appears then, starts turning.
  watch(icons, startWhileSpinning, { flush: 'post' })
  onBeforeUnmount(() => {
    disposed = true
    for (const animation of animations) {
      animation.cancel()
    }
  })

  return { rotating, onClick }
}

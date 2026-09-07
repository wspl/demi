import { onBeforeUnmount, ref, watch, type Ref } from 'vue'

export interface ButtonIconSpinProps {
  spinning?: boolean
  spinOnClick?: boolean
  disabled?: boolean
}

/** Finish each revolution before stopping, including when work finishes early. */
export function useButtonIconSpin(root: Ref<HTMLElement | null>, props: ButtonIconSpinProps, onEnd: () => void) {
  const rotating = ref(false)
  let animations: Animation[] = []
  let disposed = false

  async function start() {
    if (rotating.value || disposed || !root.value) return
    const icons = root.value.querySelectorAll(':scope > svg')
    if (!icons.length) return
    rotating.value = true
    try {
      do {
        const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
        animations = Array.from(icons, (icon) => icon.animate(
          [{ transform: 'rotate(0deg)' }, { transform: `rotate(${reduceMotion ? 0 : 360}deg)` }],
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

  watch(() => props.spinning, (spinning) => { if (spinning) void start() }, { flush: 'post', immediate: true })
  watch(root, () => { if (props.spinning) void start() }, { flush: 'post' })
  onBeforeUnmount(() => {
    disposed = true
    for (const animation of animations) animation.cancel()
  })

  return { rotating, onClick: () => { if (props.spinOnClick && !props.disabled) void start() } }
}

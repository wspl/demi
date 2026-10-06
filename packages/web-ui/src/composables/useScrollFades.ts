import { shallowRef, toValue, watch, type MaybeRefOrGetter } from 'vue'
import { useEventListener, useMutationObserver, useResizeObserver } from '@vueuse/core'

/** The sides of a sideways scroller that hide part of its content. */
type HiddenSide = 'start' | 'end'

/**
 * Marks a scroller with the sides it hides content on, which the
 * `[data-scroll-fade]` rule in base.css fades. Less than a pixel either way
 * is the rounding of a box that fits.
 */
function markHiddenSides(scroller: HTMLElement): void {
  // A right-to-left scroller counts its offset down from zero.
  const offset = Math.abs(scroller.scrollLeft)
  const hidden = scroller.scrollWidth - scroller.clientWidth
  const sides: HiddenSide[] = []
  if (offset >= 1)
    sides.push('start')
  if (offset < hidden - 1)
    sides.push('end')
  const value = sides.join(' ')
  if (scroller.dataset['scrollFade'] !== value)
    scroller.dataset['scrollFade'] = value
}

/**
 * Edge fades on the sideways scrollers under `root` that match `selector`: a
 * scroller fades its content out at each side where more of it is hidden,
 * the end side at rest, the start side once it has scrolled, and never on a
 * scroller whose content fits. The scrollers may come and go with every
 * render of `root`; each is marked as it arrives, and again as it scrolls
 * and as its box changes size.
 */
export function useScrollFades(root: MaybeRefOrGetter<HTMLElement | null | undefined>, selector: string): void {
  const scrollers = shallowRef<HTMLElement[]>([])

  function collect(): void {
    const element = toValue(root)
    scrollers.value = element ? [...element.querySelectorAll<HTMLElement>(selector)] : []
  }

  // A box starts being observed with a first report of its size, which marks
  // a scroller as it arrives.
  useResizeObserver(scrollers, (entries) => {
    for (const entry of entries) {
      if (entry.target instanceof HTMLElement)
        markHiddenSides(entry.target)
    }
  })

  watch(() => toValue(root), collect, { immediate: true, flush: 'post' })
  useMutationObserver(root, collect, { childList: true, subtree: true })

  // A scroller's own scroll does not bubble; `root` hears it while capturing.
  useEventListener(root, 'scroll', (event: Event) => {
    if (event.target instanceof HTMLElement && event.target.matches(selector))
      markHiddenSides(event.target)
  }, { capture: true, passive: true })
}

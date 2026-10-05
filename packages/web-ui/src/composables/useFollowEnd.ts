import { watch, type Ref } from 'vue'
import { useResizeObserver, useScroll } from '@vueuse/core'

/**
 * Keeps `scroller` at the end of its `content` while `follow()` holds, as a
 * terminal follows what a command prints. Content that grows pulls the view
 * to its new end as long as the reader is at the end; a reader who scrolls up
 * stays where they are, and scrolling back to the end follows again. When
 * `follow()` turns on, or the scroller mounts while it is on, the view jumps
 * to the end.
 *
 * `content` is the element inside the scroller whose height grows: the
 * scroller's own box stays at its bound while its content grows past it.
 */
export function useFollowEnd(
  scroller: Readonly<Ref<HTMLElement | undefined>>,
  content: Readonly<Ref<HTMLElement | undefined>>,
  follow: () => boolean,
): void {
  // Set by the scroll events alone, so it still says where the reader left
  // the view when the content grows beneath it.
  const { arrivedState } = useScroll(scroller)

  function scrollToEnd(el: HTMLElement): void {
    el.scrollTop = el.scrollHeight
  }

  useResizeObserver(content, () => {
    const el = scroller.value
    if (el && follow() && arrivedState.bottom) {
      scrollToEnd(el)
    }
  })

  watch(
    () => (follow() ? scroller.value : undefined),
    (el) => {
      if (el) {
        scrollToEnd(el)
      }
    },
    { immediate: true, flush: 'post' },
  )
}

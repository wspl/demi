import { onScopeDispose, toValue, type MaybeRefOrGetter } from 'vue'
import { useEventListener } from '@vueuse/core'
import { computeScrollbarAxis, type ScrollbarAxisOutput } from '../ui/scrollbar-axis'

/** The thumb as a `ScrollArea` draws it: 2px from the edges, 6px thick, never shorter than 24px. */
const THUMB_INSET = 2
const THUMB_THICKNESS = 6
const THUMB_MIN = 24

/** Overflow values under which an element scrolls sideways when its content is wider. */
const SCROLLING_OVERFLOW = new Set(['auto', 'scroll'])

/**
 * Sideways scrollers in rendered text (a code block, a wide table or equation)
 * whose scrollbar takes no room. The stylesheet (`.markdown-body` in base.css)
 * hides their native bar and paints the thumb over the scroller's bottom edge
 * while the pointer is on it. This places that thumb, through
 * `--scroll-thumb-x` and `--scroll-thumb-size` on the scroller, whenever the
 * pointer comes onto it or it scrolls, and lets the pointer drag the thumb or
 * click beside it to move a page, as on a `ScrollArea`.
 */
export function useScrollThumbs(root: MaybeRefOrGetter<HTMLElement | null | undefined>): void {
  let drag: { scroller: HTMLElement; pointerId: number; startX: number; startLeft: number } | null = null

  /** The nearest element around `target`, inside `root`, that scrolls sideways. */
  function scrollerOf(target: EventTarget | null): HTMLElement | null {
    const container = toValue(root)
    if (!container || !(target instanceof Element))
      return null
    for (let el: Element | null = target; el && el !== container; el = el.parentElement) {
      if (el instanceof HTMLElement && SCROLLING_OVERFLOW.has(getComputedStyle(el).overflowX))
        return el
    }
    return null
  }

  function axisOf(scroller: HTMLElement): ScrollbarAxisOutput {
    return computeScrollbarAxis({
      viewportSize: scroller.clientWidth,
      scrollSize: scroller.scrollWidth,
      scrollOffset: scroller.scrollLeft,
      trackSize: scroller.clientWidth - 2 * THUMB_INSET,
      minThumbSize: THUMB_MIN,
    })
  }

  // A scroller with nothing to scroll has no thumb: without the variables it is zero long.
  function paint(scroller: HTMLElement): void {
    const axis = axisOf(scroller)
    if (!axis.scrollable) {
      scroller.style.removeProperty('--scroll-thumb-x')
      scroller.style.removeProperty('--scroll-thumb-size')
      return
    }
    scroller.style.setProperty('--scroll-thumb-x', `${THUMB_INSET + axis.thumbOffset}px`)
    scroller.style.setProperty('--scroll-thumb-size', `${axis.thumbSize}px`)
  }

  useEventListener(root, 'pointerover', (event: PointerEvent) => {
    const scroller = scrollerOf(event.target)
    if (scroller)
      paint(scroller)
  })

  // A scroll event does not bubble; the root hears its scrollers' while capturing.
  useEventListener(root, 'scroll', (event: Event) => {
    const scroller = scrollerOf(event.target)
    if (scroller && scroller === event.target)
      paint(scroller)
  }, { capture: true, passive: true })

  // A touch scrolls the content itself; the thumb is never shown to it.
  useEventListener(root, 'pointerdown', (event: PointerEvent) => {
    if (event.button !== 0 || event.pointerType === 'touch')
      return
    const scroller = scrollerOf(event.target)
    if (!scroller)
      return
    const rect = scroller.getBoundingClientRect()
    const bottom = rect.top + scroller.clientTop + scroller.clientHeight
    if (event.clientY < bottom - THUMB_INSET - THUMB_THICKNESS || event.clientY > bottom)
      return
    const axis = axisOf(scroller)
    if (!axis.scrollable)
      return
    event.preventDefault()
    const thumbStart = rect.left + scroller.clientLeft + THUMB_INSET + axis.thumbOffset
    if (event.clientX < thumbStart || event.clientX > thumbStart + axis.thumbSize) {
      const direction = event.clientX < thumbStart ? -1 : 1
      scroller.scrollBy({ left: direction * scroller.clientWidth, behavior: 'smooth' })
      return
    }
    scroller.setPointerCapture(event.pointerId)
    drag = { scroller, pointerId: event.pointerId, startX: event.clientX, startLeft: scroller.scrollLeft }
    scroller.setAttribute('data-scroll-thumb', 'drag')
    document.addEventListener('lostpointercapture', endDrag, true)
  })

  // Captured, the drag's moves go to the scroller and bubble here.
  useEventListener(root, 'pointermove', (event: PointerEvent) => {
    if (!drag || event.pointerId !== drag.pointerId)
      return
    const { scroller } = drag
    const range = scroller.clientWidth - 2 * THUMB_INSET - axisOf(scroller).thumbSize
    if (range <= 0)
      return
    const scrollRange = scroller.scrollWidth - scroller.clientWidth
    scroller.scrollLeft = drag.startLeft + ((event.clientX - drag.startX) / range) * scrollRange
  })

  // Release, cancel and a scroller removed by a new render all end the capture, and with it
  // the drag. The document hears each: a removed scroller's loss is sent to the document.
  function endDrag(event: PointerEvent): void {
    if (!drag || event.pointerId !== drag.pointerId)
      return
    drag.scroller.removeAttribute('data-scroll-thumb')
    drag = null
    document.removeEventListener('lostpointercapture', endDrag, true)
  }

  onScopeDispose(() => {
    document.removeEventListener('lostpointercapture', endDrag, true)
  })
}

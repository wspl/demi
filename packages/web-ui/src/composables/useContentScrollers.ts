import { onScopeDispose, type MaybeRefOrGetter } from 'vue'
import { useEventListener } from '@vueuse/core'
import type { OverlayScrollbars } from 'overlayscrollbars'
import { attachScrollbars } from '../ui/scrollbars'
import { useScrollFades } from './useScrollFades'

/** The sideways scrollers of rendered Markdown: a code block, a wide table or equation. */
const CONTENT_SCROLLERS = '.markdown-body :is(pre, .table-scroll, .katex-display)'

/**
 * The sideways scrollers of rendered text under `root`, which a render
 * replaces wholesale: each fades out at a side that hides more of its
 * content, so a reader sees there is more before any bar shows, and has an
 * overlay scrollbar. The bars are never set up in advance: a scroller gets
 * its bar as the pointer first comes onto it, which is when the bar would
 * first show. Bars of scrollers a render has removed are destroyed then too,
 * and the rest when the scope goes.
 */
export function useContentScrollers(root: MaybeRefOrGetter<HTMLElement | null | undefined>): void {
  useScrollFades(root, CONTENT_SCROLLERS)

  const attached = new Map<HTMLElement, OverlayScrollbars>()

  function dropRemoved(): void {
    for (const [scroller, instance] of attached) {
      if (scroller.isConnected)
        continue
      instance.destroy()
      attached.delete(scroller)
    }
  }

  // While capturing, before the event reaches the scroller: the bar set up now
  // hears this same event arrive and shows.
  useEventListener(root, 'pointerover', (event: PointerEvent) => {
    const target = event.target instanceof Element ? event.target : null
    const scroller = target?.closest<HTMLElement>(CONTENT_SCROLLERS)
    if (!scroller || attached.has(scroller))
      return
    dropRemoved()
    attached.set(scroller, attachScrollbars(scroller, 'x'))
  }, { capture: true, passive: true })

  onScopeDispose(() => {
    for (const instance of attached.values())
      instance.destroy()
    attached.clear()
  })
}

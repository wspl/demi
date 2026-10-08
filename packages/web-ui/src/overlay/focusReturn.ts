import { createSharedComposable, useEventListener } from '@vueuse/core'

/**
 * What had the focus before it went to nothing, as a press on something that
 * takes no focus (an icon button, a sidebar row) sends it, until anything
 * takes the focus again. One pair of listeners for the whole page.
 */
const useFocusLostToNothing = createSharedComposable(() => {
  let lost: HTMLElement | null = null
  useEventListener(document, 'focusout', (event: FocusEvent) => {
    lost = event.relatedTarget === null && event.target instanceof HTMLElement ? event.target : null
  })
  useEventListener(document, 'focusin', () => {
    lost = null
  })
  return () => (lost?.isConnected ? lost : null)
})

/**
 * Gives the focus back to what opened a layer (a dialog, a menu) when it
 * closes, as WAI-ARIA and macOS do. The opener is what had the focus as the
 * layer opened; when the press that opened it took the focus to nothing, it
 * is what had the focus before that press, such as the message field under
 * the composer's + button. The focus goes back only while the layer holds it
 * (`holds`) or nothing does: a close that put it on another control leaves it
 * there.
 */
export function useFocusReturn(holds: (el: Element) => boolean): {
  /** Remembers the opener; called as the layer opens, before it takes the focus. */
  open: () => void
  /** Gives the focus back to the opener, or, with `giveBack` false, only forgets it. */
  close: (giveBack?: boolean) => void
} {
  const lostToNothing = useFocusLostToNothing()
  let opener: HTMLElement | null = null

  function open(): void {
    const active = document.activeElement
    opener = active instanceof HTMLElement && active !== document.body ? active : lostToNothing()
  }

  function close(giveBack = true): void {
    const target = opener
    opener = null
    if (!giveBack || !target?.isConnected)
      return
    const active = document.activeElement
    if (active !== null && active !== document.body && !holds(active))
      return
    target.focus({ preventScroll: true })
  }

  return { open, close }
}

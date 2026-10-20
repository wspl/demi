import { onKeyStroke } from '@vueuse/core'
import { FIELD } from '../ui/autofocus'

/** What a layer needs to know of an Escape. */
export interface EscapeKey {
  defaultPrevented: boolean
  isComposing: boolean
  /**
   * The element the key went to, the one with the focus; none for the page
   * itself.
   */
  target: { closest(selectors: string): unknown } | null
}

export interface EscapeCloseOptions {
  /**
   * The layer sits beside fields the user types in (a window over the
   * composer, a terminal): an Escape in one is the field's own, never the
   * layer's. A modal layer leaves this off, since Escape in its own field,
   * with nothing to revert, is how a macOS sheet is cancelled.
   */
  ignoreFields?: boolean
}

/** The key of a keydown, as `escapeCloses` reads it. */
export function escapeKey(event: KeyboardEvent): EscapeKey {
  return {
    defaultPrevented: event.defaultPrevented,
    isComposing: event.isComposing,
    target: event.target instanceof Element ? event.target : null,
  }
}

/**
 * Whether an Escape closes a layer. It does not when something used it first
 * (it called preventDefault, as a field reverting its edit does) or when an
 * input method composes, where Escape cancels the candidate; with
 * `ignoreFields`, nor when the focus is in a field the user types in.
 */
export function escapeCloses(key: EscapeKey, options: EscapeCloseOptions = {}): boolean {
  if (key.defaultPrevented || key.isComposing)
    return false
  return !options.ignoreFields || key.target?.closest(FIELD) == null
}

/**
 * Escape closes the topmost layer: `isTop` says whether this layer is open
 * and on top (the overlay store's `isTop`, the window stack's newest, with
 * no menu or dialog over it), and `close` closes it. The key it takes is
 * used up, so no layer beneath hears it.
 */
export function useEscapeClose(isTop: () => boolean, close: () => void, options: EscapeCloseOptions = {}): void {
  onKeyStroke('Escape', (event) => {
    if (!isTop() || !escapeCloses(escapeKey(event), options))
      return
    event.preventDefault()
    close()
  })
}

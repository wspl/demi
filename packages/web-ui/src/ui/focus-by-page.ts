/**
 * Moves the focus as the page does, not as the user did: a closed window or
 * menu giving it back to what opened it, a field taking it on opening. The
 * focus ring shows only where the keyboard moved the focus, so none is drawn
 * (`docs/product/web-application.md` § Keyboard). Escape closing a window is
 * a key press, and the browser would match `:focus-visible` on what the page
 * focuses next; `focusVisible: false` (Chrome 145, Safari 18.4, Firefox 104)
 * says the page moved it. Every focus the page moves goes through here.
 */
export function focusByPage(target: Pick<HTMLElement, 'focus'>, options: FocusOptions = {}): void {
  target.focus({ ...options, focusVisible: false })
}

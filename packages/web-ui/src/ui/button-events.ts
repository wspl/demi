/** Suppress activation while preserving navigation keys such as Tab. */
export function blockUnavailableButtonEvent(
  event: Event,
  unavailable?: boolean,
): void {
  if (!unavailable) {
    return
  }
  if (event.type === 'keydown') {
    const key = (event as KeyboardEvent).key
    if (key !== 'Enter' && key !== ' ') {
      return
    }
  }
  event.preventDefault()
  event.stopImmediatePropagation()
}

/** Elements whose Space went down and has not come up: a native button clicks as Space is released. */
const spaceDown = new WeakSet<EventTarget>()

/**
 * Return and Space press the button-like element that has the focus, as they press a
 * native button: Return as it goes down, Space as it comes up (and the page does not
 * scroll as it goes down).
 *
 * Bind it to both `keydown` and `keyup`. An element that is `aria-disabled` is not
 * pressed.
 */
export function pressOnKey(event: KeyboardEvent): void {
  const target = event.currentTarget
  if (event.target !== target || !(target instanceof HTMLElement) || (event.key !== 'Enter' && event.key !== ' ')) {
    return
  }
  if (target.getAttribute('aria-disabled') === 'true') {
    return
  }
  if (event.key === ' ') {
    event.preventDefault()
    if (event.type === 'keydown') {
      spaceDown.add(target)
    } else if (spaceDown.delete(target)) {
      target.click()
    }
    return
  }
  if (event.type === 'keydown') {
    event.preventDefault()
    target.click()
  }
}

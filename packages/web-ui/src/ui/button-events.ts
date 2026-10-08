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

/** Return and Space press the button-like element that has the focus, as they press a native button. */
export function pressOnKey(event: KeyboardEvent): void {
  const target = event.currentTarget
  if (event.target !== target || !(target instanceof HTMLElement) || (event.key !== 'Enter' && event.key !== ' ')) {
    return
  }
  event.preventDefault()
  target.click()
}

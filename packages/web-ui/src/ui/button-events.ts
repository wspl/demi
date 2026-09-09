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

/**
 * Whether a key belongs to an input method's composition: the Enter that
 * commits a Chinese or Japanese candidate, the Escape that drops it, a dead
 * key's accent. Chrome marks such a key `isComposing`; Safari ends the
 * composition before the committing key's `keydown`, which it marks only
 * with key code 229.
 */
export function composingKey(event: { key: string; keyCode: number; isComposing: boolean }): boolean {
  return event.isComposing || event.keyCode === 229 || event.key === 'Dead'
}

/**
 * Keeps a composition's key to the field it is typed in, for a listener in
 * the capture phase on the field's frame: no handler on the field or around
 * it sees the key, so a candidate's Enter submits nothing and its Escape
 * closes nothing. The input method still acts on it: only the page's
 * listeners, the field's own among them, stop seeing it.
 */
export function keepComposition(event: { key: string; keyCode: number; isComposing: boolean; stopPropagation(): void }): void {
  if (composingKey(event)) {
    event.stopPropagation()
  }
}

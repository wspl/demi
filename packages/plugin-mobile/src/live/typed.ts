/**
 * What the device's focused field gets of the view's own input element:
 *
 * - A key's own character goes with the key, whose default the view
 *   prevents.
 * - An input method's text goes with its composition's end.
 * - What is left is text put in without a key, as the Character Viewer,
 *   an emoji picker or dictation puts it in: committed text, never a
 *   composition's draft.
 */
export interface Inserted {
  readonly isComposing: boolean
  readonly inputType: string
  readonly data: string | null
}

/** The text an input event inserted, or none. */
export function insertedText(event: Inserted): string | undefined {
  return !event.isComposing && event.inputType === 'insertText' && event.data ? event.data : undefined
}

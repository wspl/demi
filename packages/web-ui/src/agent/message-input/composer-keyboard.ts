/** Enter submits; Shift+Enter and active IME composition keep editing. */
export function shouldSubmitFromEditorKeydown(
  event: Pick<KeyboardEvent, 'isComposing' | 'key' | 'shiftKey'>,
): boolean {
  return !event.isComposing && event.key === 'Enter' && !event.shiftKey
}

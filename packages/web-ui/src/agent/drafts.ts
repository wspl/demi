import { isMacOS } from '@tiptap/core'

/**
 * One saved draft as the list under the composer shows it
 * (`product.md` § Drafts): the message's first line and how many files it
 * carries. The list shows no time.
 */
export interface DraftRow {
  id: string
  /** The message as one line, as `draftPreview` builds it. */
  preview: string
  attachmentCount: number
}

/** The entry's text under the composer: `1 Draft`, `3 Drafts`. */
export function draftsLabel(count: number): string {
  return count === 1 ? '1 Draft' : `${count} Drafts`
}

/** The key that saves the message as a draft, ⌘S on macOS and Ctrl+S elsewhere. */
export function saveDraftKeys(): string {
  return isMacOS() ? '⌘S' : 'Ctrl+S'
}

/** Whether a key press is the Save as Draft shortcut. */
export function asksSaveDraft(event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>): boolean {
  const modifier = isMacOS() ? event.metaKey : event.ctrlKey
  return modifier && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 's'
}

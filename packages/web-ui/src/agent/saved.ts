import { isMacOS } from '@tiptap/core'
import { ATTACHMENT_MARK } from '../markdown/user-markdown'
import { t } from './locales/text'

/**
 * Where a message of the list stands: kept by the server, on its way there,
 * or refused, which the list shows as Not Saved with Retry and Delete
 * (`product.md` § Saved for later).
 */
export type SavedRowState = 'saved' | 'saving' | 'notSaved'

/**
 * One message saved for later as the list under the composer shows it
 * (`product.md` § Saved for later): the message's first line and how many
 * files it carries. The list shows no time.
 */
export interface SavedRow {
  id: string
  /** The message as one line, as `savedPreview` builds it. */
  preview: string
  attachmentCount: number
  state: SavedRowState
}

/**
 * A saved message's row text: its first line that holds anything, without
 * its files, which the row's count stands for. A file between words leaves
 * one space; one before a punctuation mark or at a line's end leaves none.
 */
export function savedPreview(markdown: string): string {
  const marks = new RegExp(`[ \\t]*${ATTACHMENT_MARK}+[ \\t]*`, 'gu')
  const text = markdown.replace(marks, (found: string, offset: number, whole: string) => {
    const next = whole[offset + found.length] ?? '\n'
    const previous = whole[offset - 1] ?? '\n'
    return /[\s.,;:!?)\]]/u.test(next) || previous === '\n' ? '' : ' '
  })
  return text.split('\n').map((line) => line.trim()).find((line) => line !== '') ?? ''
}

/** The entry's text under the composer: `1 Saved`, `3 Saved`. */
export function savedLabel(count: number): string {
  return t('composer.saved.count', { count })
}

/** The key that saves the message for later, ⌘S on macOS and Ctrl+S elsewhere. */
export function saveForLaterKeys(): string {
  return isMacOS() ? '⌘S' : 'Ctrl+S'
}

/** Whether a key press is the Save for Later shortcut. */
export function asksSaveForLater(event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'>): boolean {
  const modifier = isMacOS() ? event.metaKey : event.ctrlKey
  return modifier && !event.altKey && !event.shiftKey && event.key.toLowerCase() === 's'
}

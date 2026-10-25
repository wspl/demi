import { ATTACHMENT_MARK } from '../markdown/user-markdown'
import { matchesShortcut, shortcutFor } from '../ui/shortcut'
import { t } from './locales/text'
import type { MessageCapsule } from './message-editor/capsules'

/** Where a message of the list stands: kept by the server, or on its way there. */
export type SavedRowState = 'saved' | 'saving'

/** A run of a saved message's row: text, or a file as its capsule. */
export type SavedPart = { text: string } | { capsule: MessageCapsule }

/**
 * One message saved for later as the list under the composer shows it: the
 * message on one line, each file as its capsule where it stands, as the
 * composer shows it. The list shows no time.
 */
export interface SavedRow {
  id: string
  /** The message as `savedParts` lays it on one line. */
  parts: SavedPart[]
  state: SavedRowState
}

/**
 * A saved message on one line: its lines trimmed and joined by a space, the
 * blank ones dropped, and each mark of `markdown` as the capsule of the file
 * it stands for, in order. Files without a mark follow the text, as a stored
 * message's do (`storedMessageContent`). `markdown` may be the start of the
 * message alone (`cut`), as the server lists it: an ellipsis then ends it,
 * before the files whose marks lie past it.
 */
export function savedParts(markdown: string, capsules: readonly MessageCapsule[], cut = false): SavedPart[] {
  const line = markdown.split('\n').map((text) => text.trim()).filter((text) => text !== '').join(' ')
  const texts = line.split(ATTACHMENT_MARK)
  return [
    ...texts.flatMap((text, index): SavedPart[] => {
      const capsule = index > 0 ? capsules[index - 1] : undefined
      return [...(capsule ? [{ capsule }] : []), ...(text ? [{ text }] : [])]
    }),
    ...(cut ? [{ text: ' …' }] : []),
    ...capsules.slice(texts.length - 1).map((capsule) => ({ capsule })),
  ]
}

/** The entry's text under the composer: `1 Saved`, `3 Saved`. */
export function savedLabel(count: number): string {
  return t('composer.saved.count', { count })
}

/** The key that saves the message for later, in the page's set. */
export function saveForLaterKeys(): string {
  return shortcutFor({ mac: '⌘S', pc: 'Ctrl+S' })
}

/** Whether a key press is the Save for Later shortcut. */
export function asksSaveForLater(event: KeyboardEvent): boolean {
  return matchesShortcut(event, saveForLaterKeys())
}

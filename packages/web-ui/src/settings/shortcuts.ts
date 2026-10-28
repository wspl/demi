import type { SentenceText } from '../ui/ui-text'
import { holdsCommandModifier, shortcutSet, type ShortcutSetId } from '../ui/shortcut'
import { t } from './locales/text'
import type { SettingsKeyBinding } from './types'

/**
 * The app's keyboard shortcuts and the keys each has until the user changes them, one
 * list for each set, in the set's notation (`ui/shortcut.ts`). The PC's keys are the
 * Mac's with Ctrl for ⌘.
 *
 * - New conversation is ⇧⌘O, as in ChatGPT: ⌘N is the browser's new window, which a
 *   page never receives.
 * - Search is ⌘K, as in Slack, Linear and ChatGPT.
 * - Search in files is ⇧⌘F, as in VS Code and Zed.
 * - Find is ⌘F, and finds where the user is, as VS Code's finds in the part that
 *   has the focus: Search in files in Files, the conversations' search elsewhere.
 *   A text with a find of its own, as a file's, finds there first.
 * - Workspace 1 to 9 are ⌃1 to ⌃9, as Arc's spaces are, and Ctrl+Shift+1 to
 *   Ctrl+Shift+9 on a PC: ⌘1 to ⌘9 and Ctrl+1 to Ctrl+9 are the browser's tabs,
 *   which a page never takes, Alt+1 to Alt+9 are its tabs on Linux, and a PC
 *   has no ⌃ apart from Ctrl.
 * - The sidebar is ⌘\, as in Notion, whose editor keeps ⌘B for bold and ⇧⌘S for
 *   strikethrough as the composer does: the focused composer takes its keys first, so
 *   a sidebar shortcut on either would never run while it has the focus.
 */
const APP_SHORTCUTS = {
  mac: [
    { id: 'new', keys: '⇧⌘O' },
    { id: 'search', keys: '⌘K' },
    { id: 'sidebar', keys: '⌘\\' },
    { id: 'settings', keys: '⌘,' },
    { id: 'searchFiles', keys: '⇧⌘F' },
    { id: 'find', keys: '⌘F' },
    { id: 'workspace1', keys: '⌃1' },
    { id: 'workspace2', keys: '⌃2' },
    { id: 'workspace3', keys: '⌃3' },
    { id: 'workspace4', keys: '⌃4' },
    { id: 'workspace5', keys: '⌃5' },
    { id: 'workspace6', keys: '⌃6' },
    { id: 'workspace7', keys: '⌃7' },
    { id: 'workspace8', keys: '⌃8' },
    { id: 'workspace9', keys: '⌃9' },
  ],
  pc: [
    { id: 'new', keys: 'Ctrl+Shift+O' },
    { id: 'search', keys: 'Ctrl+K' },
    { id: 'sidebar', keys: 'Ctrl+\\' },
    { id: 'settings', keys: 'Ctrl+,' },
    { id: 'searchFiles', keys: 'Ctrl+Shift+F' },
    { id: 'find', keys: 'Ctrl+F' },
    { id: 'workspace1', keys: 'Ctrl+Shift+1' },
    { id: 'workspace2', keys: 'Ctrl+Shift+2' },
    { id: 'workspace3', keys: 'Ctrl+Shift+3' },
    { id: 'workspace4', keys: 'Ctrl+Shift+4' },
    { id: 'workspace5', keys: 'Ctrl+Shift+5' },
    { id: 'workspace6', keys: 'Ctrl+Shift+6' },
    { id: 'workspace7', keys: 'Ctrl+Shift+7' },
    { id: 'workspace8', keys: 'Ctrl+Shift+8' },
    { id: 'workspace9', keys: 'Ctrl+Shift+9' },
  ],
} as const

export type AppShortcutId = (typeof APP_SHORTCUTS)['mac'][number]['id']

/** The shortcuts of a set with the keys each has by default; the page's set unless one is named. */
export function appShortcuts(set: ShortcutSetId = shortcutSet()): { id: AppShortcutId; keys: string }[] {
  return APP_SHORTCUTS[set].map((shortcut) => ({ ...shortcut }))
}

/** What the shortcut `id` does, in the page's language, as its row and a refusal name it. */
export function shortcutAction(id: string): SentenceText {
  switch (id) {
    case 'new':
      return t('keyboard.actions.new')
    case 'search':
      return t('keyboard.actions.search')
    case 'sidebar':
      return t('keyboard.actions.sidebar')
    case 'settings':
      return t('keyboard.actions.settings')
    case 'searchFiles':
      return t('keyboard.actions.searchFiles')
    case 'find':
      // Its text is findHere: vue-i18n's key paths leave out a key named find.
      return t('keyboard.actions.findHere')
    default:
      return id.startsWith('workspace') ? t('keyboard.actions.workspace', { number: id.slice('workspace'.length) }) : id
  }
}

export function isAppShortcut(id: string): id is AppShortcutId {
  return APP_SHORTCUTS.mac.some((shortcut) => shortcut.id === id)
}

/**
 * Why `keys` cannot become the shortcut `id` among `bindings`, said beside
 * its row; null when they can. Keys without ⌘ or ⌃ (Ctrl on a PC) would fire
 * while the user types, and keys another action holds would run two actions at once.
 */
export function shortcutRefusal(
  keys: string,
  id: string,
  bindings: readonly SettingsKeyBinding[],
): SentenceText | null {
  if (!holdsCommandModifier(keys)) {
    return t(shortcutSet() === 'mac' ? 'keyboard.typesTextMac' : 'keyboard.typesTextPc', { keys })
  }
  const taken = bindings.find((binding) => binding.id !== id && binding.keys === keys)
  return taken ? t('keyboard.taken', { keys, action: shortcutAction(taken.id) }) : null
}

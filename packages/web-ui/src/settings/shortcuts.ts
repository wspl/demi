import type { SentenceText } from '../ui/ui-text'
import type { SettingsKeyBinding } from './types'

/**
 * The app's keyboard shortcuts and the keys each has until the user changes
 * them, in the recorder's notation (`ui/shortcut.ts`). New conversation is
 * ⇧⌘O, as in ChatGPT: ⌘N is the browser's new window, which a page never
 * receives. Search is ⌘K, as in Slack, Linear and ChatGPT.
 */
export const APP_SHORTCUTS = [
  { id: 'new', action: 'New conversation', keys: '⇧⌘O' },
  { id: 'search', action: 'Search conversations', keys: '⌘K' },
  { id: 'sidebar', action: 'Toggle sidebar', keys: '⌘B' },
  { id: 'settings', action: 'Open settings', keys: '⌘,' },
] as const

export type AppShortcutId = (typeof APP_SHORTCUTS)[number]['id']

export function isAppShortcut(id: string): id is AppShortcutId {
  return APP_SHORTCUTS.some((shortcut) => shortcut.id === id)
}

/** Whether keys hold ⌘ or ⌃, which no typing produces: a key alone, or with ⇧ or ⌥, types a character. */
export function hasCommandModifier(keys: string): boolean {
  return keys.includes('⌘') || keys.includes('⌃')
}

/**
 * Why `keys` cannot become the shortcut `id` among `bindings`, said beside
 * its row; null when they can. Keys without ⌘ or ⌃ would fire while the
 * user types, and keys another action holds would run two actions at once.
 */
export function shortcutRefusal(
  keys: string,
  id: string,
  bindings: readonly SettingsKeyBinding[],
): SentenceText | null {
  if (!hasCommandModifier(keys)) {
    return `${keys} types text. Use ⌘ or ⌃ with a key.`
  }
  const taken = bindings.find((binding) => binding.id !== id && binding.keys === keys)
  return taken ? `${keys} is already used by ${taken.action}.` : null
}

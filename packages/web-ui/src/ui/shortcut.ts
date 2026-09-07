/**
 * One notation for keyboard shortcuts, shared by the recorder that writes it and
 * the app that listens for it: modifiers in the fixed order ⌃ ⌥ ⇧ ⌘, then one key
 * cap (`⌘⇧O`, `⏎`, `⌥←`). Letters and digits come from the physical key, so ⇧
 * never turns `k` into `K` twice or `,` into `<`.
 */
const SPECIAL: Record<string, string> = {
  Enter: '⏎',
  Escape: '⎋',
  Backspace: '⌫',
  Delete: '⌦',
  Tab: '⇥',
  ' ': '␣',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
}
const MODIFIERS = new Set(['Meta', 'Control', 'Alt', 'Shift'])

export function shortcutModifiers(event: KeyboardEvent): string {
  return `${event.ctrlKey ? '⌃' : ''}${event.altKey ? '⌥' : ''}${event.shiftKey ? '⇧' : ''}${event.metaKey ? '⌘' : ''}`
}

/** The cap for the key an event names; empty while only modifiers are down. */
export function shortcutKeyCap(event: KeyboardEvent): string {
  if (MODIFIERS.has(event.key)) return ''
  if (SPECIAL[event.key]) return SPECIAL[event.key]
  if (event.code.startsWith('Key')) return event.code.slice(3)
  if (event.code.startsWith('Digit')) return event.code.slice(5)
  return event.key.length === 1 ? event.key.toUpperCase() : event.key
}

/** The whole shortcut an event spells, or empty while only modifiers are down. */
export function formatShortcut(event: KeyboardEvent): string {
  const cap = shortcutKeyCap(event)
  return cap ? shortcutModifiers(event) + cap : ''
}

/** Whether an event is exactly this shortcut: the same modifiers, the same key. */
export function matchesShortcut(event: KeyboardEvent, keys: string): boolean {
  return keys !== '' && formatShortcut(event) === keys
}

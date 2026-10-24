import { clientPlatform } from '@demicodes/utils'

/**
 * Keyboard shortcuts come in two sets, the Mac's and the PC's. A page uses the set of
 * the system it runs on, and the one notation of that set is written by the recorder,
 * stored, matched against the keys an event holds and shown, so a shortcut is never
 * translated between sets.
 *
 * - Mac: modifiers in the order ⌃ ⌥ ⇧ ⌘, then one key cap (`⇧⌘O`, `⏎`, `⌥←`).
 * - PC (Windows and Linux): modifiers in the order Ctrl, Shift, Alt, then one key cap,
 *   joined by `+` (`Ctrl+Shift+O`, `Enter`, `Alt+Left`). The Windows or Super key is
 *   part of no shortcut, as the system keeps it.
 *
 * Letters, digits and punctuation come from the physical key, named as on a US
 * keyboard, so ⇧ never turns `k` into `K` twice or `,` into `<`, nor ⌥ `,` into `≤`.
 *
 * The module is the page's one answer to which system's keys it takes: the user's
 * shortcuts, the app's fixed ones (`shortcutFor`), and the command key a click holds
 * (`withCommandKey`) all follow `shortcutSet`. The editors' own keymaps, such as bold
 * in the composer, follow their libraries' `Mod`.
 */
export type ShortcutSetId = 'mac' | 'pc'

interface Notation {
  /** The modifiers in the order they are written, each with the flag of the event that holds it. */
  modifiers: readonly (readonly [string, 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'])[]
  separator: string
  /** The caps of the keys that have a name, by `KeyboardEvent.key`. */
  special: Record<string, string>
}

const NOTATIONS: Record<ShortcutSetId, Notation> = {
  mac: {
    modifiers: [['⌃', 'ctrlKey'], ['⌥', 'altKey'], ['⇧', 'shiftKey'], ['⌘', 'metaKey']],
    separator: '',
    special: {
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
    },
  },
  pc: {
    // No Windows or Super key: the system keeps it.
    modifiers: [['Ctrl', 'ctrlKey'], ['Shift', 'shiftKey'], ['Alt', 'altKey']],
    separator: '+',
    special: {
      Enter: 'Enter',
      Escape: 'Esc',
      Backspace: 'Backspace',
      Delete: 'Delete',
      Tab: 'Tab',
      ' ': 'Space',
      ArrowUp: 'Up',
      ArrowDown: 'Down',
      ArrowLeft: 'Left',
      ArrowRight: 'Right',
    },
  },
}
const MODIFIER_KEYS = new Set(['Meta', 'Control', 'Alt', 'Shift'])
/** The caps of the punctuation keys, by `KeyboardEvent.code`, unshifted. */
const PUNCTUATION: Record<string, string> = {
  Backquote: '`',
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
}

/** The set the page uses: the Mac's on macOS, the PC's on every other system. */
export function shortcutSet(): ShortcutSetId {
  return clientPlatform(navigator) === 'mac' ? 'mac' : 'pc'
}

/**
 * Whether an event holds the set's command key, ⌘ on a Mac and Ctrl on a PC, as a
 * click that adds a row to a selection or opens a link does in Finder and Explorer. A
 * Mac's Control-click is a right-click, and a PC's Windows key is the system's.
 */
export function withCommandKey(event: Pick<MouseEvent, 'metaKey' | 'ctrlKey'>): boolean {
  return shortcutSet() === 'mac' ? event.metaKey : event.ctrlKey
}

/** The cap of a key that has a name, by `KeyboardEvent.key`, in the page's set: `⏎` or `Enter`. */
export function namedKeyCap(key: string): string {
  return NOTATIONS[shortcutSet()].special[key] ?? key
}

/** The modifiers an event holds, in the order they are written. */
function heldModifiers(notation: Notation, event: KeyboardEvent): string[] {
  return notation.modifiers.filter(([, flag]) => event[flag]).map(([name]) => name)
}

/** The modifiers an event holds, written in the page's set. */
export function shortcutModifiers(event: KeyboardEvent): string {
  const notation = NOTATIONS[shortcutSet()]
  return heldModifiers(notation, event).join(notation.separator)
}

/** The cap for the key an event names; empty while only modifiers are down. */
function shortcutKeyCap(event: KeyboardEvent): string {
  if (MODIFIER_KEYS.has(event.key))
    return ''
  const special = NOTATIONS[shortcutSet()].special[event.key]
  if (special)
    return special
  if (event.code.startsWith('Key'))
    return event.code.slice(3)
  if (event.code.startsWith('Digit'))
    return event.code.slice(5)
  if (PUNCTUATION[event.code])
    return PUNCTUATION[event.code]
  return event.key.length === 1 ? event.key.toUpperCase() : event.key
}

/**
 * The whole shortcut an event spells in the page's set, or empty while only modifiers
 * are down, and on a PC while the Windows key is, which no shortcut holds.
 */
export function formatShortcut(event: KeyboardEvent): string {
  const set = shortcutSet()
  const cap = shortcutKeyCap(event)
  if (!cap || (set === 'pc' && event.metaKey))
    return ''
  const notation = NOTATIONS[set]
  return [...heldModifiers(notation, event), cap].join(notation.separator)
}

/** Whether an event is exactly this shortcut: the same modifiers, the same key. */
export function matchesShortcut(event: KeyboardEvent, keys: string): boolean {
  return keys !== '' && formatShortcut(event) === keys
}

/** A shortcut of the page's set split into the modifiers it holds and its key cap, which is empty for modifiers alone. */
function splitShortcut(keys: string): { modifiers: string[]; cap: string } {
  const notation = NOTATIONS[shortcutSet()]
  // A PC's key can be `+` itself (`Ctrl++`), which the separator would cut.
  let parts: string[]
  if (!notation.separator) {
    parts = Array.from(keys)
  } else if (keys.endsWith(notation.separator)) {
    const head = keys.slice(0, -1)
    parts = [...(head.endsWith(notation.separator) ? head.slice(0, -1) : head).split(notation.separator).filter(Boolean), notation.separator]
  } else {
    parts = keys.split(notation.separator)
  }
  const names = notation.modifiers.map(([name]) => name)
  const modifiers = parts.findIndex((part) => !names.includes(part))
  if (modifiers < 0)
    return { modifiers: parts, cap: '' }
  return { modifiers: parts.slice(0, modifiers), cap: parts.slice(modifiers).join(notation.separator) }
}

/** The caps a shortcut of the page's set is drawn with, one per key. */
export function shortcutCaps(keys: string): string[] {
  const { modifiers, cap } = splitShortcut(keys)
  return cap ? [...modifiers, cap] : modifiers
}

/**
 * Whether a shortcut of the page's set holds the key no typing produces: ⌘ or ⌃ on a
 * Mac, Ctrl on a PC. A key alone, or with ⇧ or ⌥, types a character.
 */
export function holdsCommandModifier(keys: string): boolean {
  const { modifiers } = splitShortcut(keys)
  return shortcutSet() === 'mac'
    ? modifiers.includes('⌘') || modifiers.includes('⌃')
    : modifiers.includes('Ctrl')
}

/** A shortcut the app fixes rather than the user sets, or a list of them, written once for each set: the page's. */
export function shortcutFor<T extends string | readonly string[]>(forms: Record<ShortcutSetId, T>): T {
  return forms[shortcutSet()]
}

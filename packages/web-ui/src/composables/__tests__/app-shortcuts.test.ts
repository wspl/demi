import { afterEach, beforeEach, expect, test } from 'bun:test'
import { effectScope } from 'vue'
import { useAppShortcuts } from '../useAppShortcuts'
import { APP_SHORTCUTS, shortcutRefusal } from '../../settings/shortcuts'

// The keys reach the app the way a browser delivers them: a keydown on the
// window, after the focused control had its turn. About 10 ms.

const realWindow = Reflect.get(globalThis, 'window')
const realDocument = Reflect.get(globalThis, 'document')
const body = { tagName: 'BODY', hasAttribute: () => false }
let ran: string[]
let stop: () => void

beforeEach(() => {
  Reflect.set(globalThis, 'window', new EventTarget())
  Reflect.set(globalThis, 'document', { body, activeElement: body })
  ran = []
})

afterEach(() => {
  stop()
  Reflect.set(globalThis, 'window', realWindow)
  Reflect.set(globalThis, 'document', realDocument)
})

function listen(bindings: readonly { id: string; keys: string }[] = APP_SHORTCUTS): void {
  const scope = effectScope()
  scope.run(() => useAppShortcuts(() => true, () => bindings, {
    new: () => ran.push('new'),
    sidebar: () => ran.push('sidebar'),
    settings: () => ran.push('settings'),
  }))
  stop = () => scope.stop()
}

/** Presses a key on the window: `taken` when the focused control used it first. */
function press(code: string, modifiers: { meta?: boolean; shift?: boolean } = {}, taken = false): void {
  const event = Object.assign(new Event('keydown', { cancelable: true }), {
    key: code.replace('Key', '').toLowerCase(),
    code,
    metaKey: modifiers.meta ?? false,
    shiftKey: modifiers.shift ?? false,
    ctrlKey: false,
    altKey: false,
  })
  if (taken) {
    event.preventDefault()
  }
  window.dispatchEvent(event)
}

/** The focus is in the composer, a field the user types in. */
function typing(): void {
  Reflect.set(document, 'activeElement', { tagName: 'DIV', hasAttribute: (name: string) => name === 'contenteditable' })
}

test('New conversation is ⇧⌘O, and runs while the user is in the composer', () => {
  listen()
  typing()
  press('KeyO', { meta: true, shift: true })
  expect(ran).toEqual(['new'])
})

test('a key the focused control used, such as ⌘B for bold, runs no shortcut', () => {
  listen()
  typing()
  press('KeyB', { meta: true }, true)
  expect(ran).toEqual([])
  press('KeyB', { meta: true })
  expect(ran).toEqual(['sidebar'])
})

test('a shortcut without ⌘ or ⌃ never runs while the user types, only outside a field', () => {
  listen([{ id: 'new', keys: 'K' }])
  typing()
  press('KeyK')
  expect(ran).toEqual([])
  Reflect.set(document, 'activeElement', body)
  press('KeyK')
  expect(ran).toEqual(['new'])
})

test('recording refuses keys that type and keys another action holds, and says why', () => {
  const bindings = APP_SHORTCUTS.map((shortcut) => ({ ...shortcut }))
  expect(shortcutRefusal('K', 'new', bindings)).toBe('K types text. Use ⌘ or ⌃ with a key.')
  expect(shortcutRefusal('⇧K', 'new', bindings)).toBe('⇧K types text. Use ⌘ or ⌃ with a key.')
  expect(shortcutRefusal('⌘B', 'new', bindings)).toBe('⌘B is already used by Toggle sidebar.')
  expect(shortcutRefusal('⌃⌥N', 'new', bindings)).toBeNull()
  // A shortcut may keep its own keys.
  expect(shortcutRefusal('⇧⌘O', 'new', bindings)).toBeNull()
})

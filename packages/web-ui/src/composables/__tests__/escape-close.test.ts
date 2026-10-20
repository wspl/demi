import { describe, expect, test } from 'bun:test'
import { escapeCloses } from '../useEscapeClose'

/**
 * An element the focus is on, which the given selectors in a selector list
 * match.
 */
function focusOn(own: string) {
  return { closest: (selectors: string) => (selectors.split(',').some((selector) => selector.trim() === own) ? {} : null) }
}
const key = { defaultPrevented: false, isComposing: false, target: focusOn('button') }

describe('Escape closes a layer', () => {
  test('with the focus on a control or on the page itself', () => {
    expect(escapeCloses(key)).toBe(true)
    expect(escapeCloses({ ...key, target: null }, { ignoreFields: true })).toBe(true)
    expect(escapeCloses(key, { ignoreFields: true })).toBe(true)
  })

  test('not when something used the key first, or an input method composes', () => {
    expect(escapeCloses({ ...key, defaultPrevented: true })).toBe(false)
    expect(escapeCloses({ ...key, isComposing: true })).toBe(false)
  })

  test.each([
    ['a text field', 'textarea'],
    ['the composer or an editor', '[contenteditable=true]'],
    ['a combo box', '[role=combobox]'],
  ])('a window beside fields is not closed by an Escape typed in %s', (_, own) => {
    expect(escapeCloses({ ...key, target: focusOn(own) }, { ignoreFields: true })).toBe(false)
  })

  test('a modal layer is closed by an Escape in its own field', () => {
    expect(escapeCloses({ ...key, target: focusOn('textarea') })).toBe(true)
  })
})

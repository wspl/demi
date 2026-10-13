import { expect, test } from 'bun:test'
import { autofillAttrs, drawnAsDots, hiddenUsername, inputType } from '../field-autofill'

// What a browser and a password manager read off a field decides whether they
// offer the saved login there (`TextInput`'s `masked` and `search`): only an
// account password is a password field; every other secret and every filter
// is a field they leave alone.

test('an account password is a password field, with nothing that hides it from the password manager', () => {
  expect(inputType('password', false, false)).toBe('password')
  expect(autofillAttrs('password', false)).toEqual({})
  expect(drawnAsDots('password', false)).toBe(false)
})

test('a revealed account password is text', () => {
  expect(inputType('password', false, true)).toBe('text')
})

test('any other secret is never a password field, hidden or revealed, and carries every password manager’s ignore switch', () => {
  expect(inputType('secret', false, false)).toBe('text')
  expect(inputType('secret', false, true)).toBe('text')
  expect(autofillAttrs('secret', false)).toEqual({
    autocomplete: 'off',
    'data-1p-ignore': true,
    'data-lpignore': 'true',
    'data-bwignore': 'true',
    'data-form-type': 'other',
  })
})

test('a secret is drawn as dots until its eye reveals it', () => {
  expect(drawnAsDots('secret', false)).toBe(true)
  expect(drawnAsDots('secret', true)).toBe(false)
  expect(drawnAsDots(undefined, false)).toBe(false)
})

test('a filter is a search field with autofill off, and a plain field is left as it is', () => {
  expect(inputType(undefined, true, false)).toBe('search')
  expect(autofillAttrs(undefined, true)).toHaveProperty('autocomplete', 'off')
  expect(inputType(undefined, false, false)).toBe('text')
  expect(autofillAttrs(undefined, false)).toEqual({})
})

test('a form that asks only for a password carries the account’s email as a user name no one sees or reaches', () => {
  const field = hiddenUsername('zan@example.com')
  expect(field.autocomplete).toBe('username')
  expect(field.value).toBe('zan@example.com')
  expect(field.tabindex).toBe(-1)
  expect(field.readonly).toBe(true)
  expect(field['aria-hidden']).toBe(true)
  expect(field.style.opacity).toBe(0)
})

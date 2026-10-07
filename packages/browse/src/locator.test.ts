import { expect, test } from 'bun:test'
import { parse } from './command'
import { parseTarget } from './locator'
import { splitWords } from './words'

test('a target is a point, or a chain of Playwright selectors, labels and test ids', () => {
  expect(parseTarget('120, 48')).toEqual({ kind: 'point', x: 120, y: 48 })
  expect(parseTarget('role=button[name="Reload"]')).toEqual({
    kind: 'locator',
    text: 'role=button[name="Reload"]',
    segments: [{ kind: 'selector', selector: 'role=button[name="Reload"]' }],
  })
  expect(parseTarget('role=dialog[name="Add Device"] >> label=Pairing code')).toMatchObject({
    segments: [
      { kind: 'selector', selector: 'role=dialog[name="Add Device"]' },
      { kind: 'label', text: 'Pairing code', exact: false },
    ],
  })
  expect(parseTarget('label="Email" >> testid=save')).toMatchObject({
    segments: [{ kind: 'label', text: 'Email', exact: true }, { kind: 'testid', id: 'save' }],
  })
  // A `>>` inside quotes is text, not a step.
  expect(parseTarget('text="a >> b"')).toMatchObject({ segments: [{ kind: 'text', text: 'a >> b', exact: true }] })
  expect(() => parseTarget('role=button >> ')).toThrow('an empty step')
})

test('text= names an element by its whole text, text~= by a part of it', () => {
  // `text=Send` must not also match "Send Feedback", as a person reads the label Send.
  expect(parseTarget('text=Send')).toMatchObject({ segments: [{ kind: 'text', text: 'Send', exact: true }] })
  expect(parseTarget('text~=Allow for')).toMatchObject({ segments: [{ kind: 'text', text: 'Allow for', exact: false }] })
  expect(parseTarget('role=dialog >> text~="Pair"')).toMatchObject({
    segments: [{ kind: 'selector', selector: 'role=dialog' }, { kind: 'text', text: 'Pair', exact: false }],
  })
})

test('a textbox locator also finds a field the page marks as a combobox', () => {
  expect(parseTarget('role=textbox[name="Search"]')).toMatchObject({
    segments: [{ kind: 'field', textbox: 'role=textbox[name="Search"]', combobox: 'role=combobox[name="Search"]' }],
  })
  expect(parseTarget('role=textbox')).toMatchObject({ segments: [{ kind: 'field', combobox: 'role=combobox' }] })
  // Another role that starts with the same letters is not a textbox.
  expect(parseTarget('role=textboxes')).toMatchObject({ segments: [{ kind: 'selector', selector: 'role=textboxes' }] })
})

test('a negative number on the command line is a number, never an option', () => {
  expect(parse(['body', '0', '-200'], {}, 'scroll').positionals).toEqual(['body', '0', '-200'])
  expect(parse(['-5,10'], {}, 'click').positionals).toEqual(['-5,10'])
  expect(parse(['--pad', '-5'], { pad: { type: 'string' } }, 'shot').values).toEqual({ pad: '-5' })
  expect(() => parse(['-x'], {}, 'scroll')).toThrow('Usage: bun browse scroll')
})

test('a command line splits at spaces outside quotes and a locator\'s brackets', () => {
  expect(splitWords('click role=button[name="Reload Page"]')).toEqual(['click', 'role=button[name="Reload Page"]'])
  expect(splitWords(`fill label=Email 'me@example.test'`)).toEqual(['fill', 'label=Email', 'me@example.test'])
  expect(splitWords('net cut "/\\/stream$/"')).toEqual(['net', 'cut', '/\\/stream$/'])
  expect(splitWords('type ""')).toEqual(['type', ''])
  expect(() => splitWords('type "open')).toThrow('unclosed')
})

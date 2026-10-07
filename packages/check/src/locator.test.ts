import { expect, test } from 'bun:test'
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
  expect(parseTarget('text="a >> b"')).toMatchObject({ segments: [{ kind: 'selector', selector: 'text="a >> b"' }] })
  expect(() => parseTarget('role=button >> ')).toThrow('an empty step')
})

test('a command line splits at spaces outside quotes and a locator\'s brackets', () => {
  expect(splitWords('click role=button[name="Reload Page"]')).toEqual(['click', 'role=button[name="Reload Page"]'])
  expect(splitWords(`fill label=Email 'me@example.test'`)).toEqual(['fill', 'label=Email', 'me@example.test'])
  expect(splitWords('net cut "/\\/stream$/"')).toEqual(['net', 'cut', '/\\/stream$/'])
  expect(splitWords('type ""')).toEqual(['type', ''])
  expect(() => splitWords('type "open')).toThrow('unclosed')
})

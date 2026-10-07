import { expect, test } from 'bun:test'
import { parse } from './command'
import { parseTarget, type Segment } from './locator'
import { splitWords } from './words'

test('a target is a point, or a chain of Playwright selectors, labels and test ids', () => {
  expect(parseTarget('120, 48')).toEqual({ kind: 'point', x: 120, y: 48 })
  expect(parseTarget('role=dialog >> label=Pairing code')).toEqual({
    kind: 'locator',
    text: 'role=dialog >> label=Pairing code',
    segments: [{ kind: 'selector', selector: 'role=dialog' }, { kind: 'label', text: 'Pairing code', exact: false }],
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

/** The one step `text` parses to. */
function only(text: string): Segment {
  const target = parseTarget(text)
  if (target.kind !== 'locator' || target.segments.length !== 1) {
    throw new Error(`${text} is not one step`)
  }
  return target.segments[0]
}

/** The accessible names the quoted name of the role step `text` matches, of `names`. */
function namesMatched(text: string, names: string[]): string[] {
  const step = only(text)
  const selector = step.kind === 'selector' ? step.selector : step.kind === 'field' ? step.textbox : ''
  // Playwright's role selector takes the name as this regular expression.
  const written = /\[name=\/(.*)\/([a-z]*)\]/.exec(selector)
  if (!written) {
    throw new Error(`${selector} names no regular expression`)
  }
  const name = new RegExp(written[1], written[2])
  return names.filter((candidate) => name.test(candidate))
}

test('a role\'s quoted name is the whole name, a trailing ellipsis aside; name~= is a part of it', () => {
  const names = ['Add Device', 'Add Device…', 'Add Device...', 'Add Devices', 'add device', 'Add Device Now']
  // A menu item reads "Add Device…"; a person names it Add Device, or as it reads.
  expect(namesMatched('role=button[name="Add Device"]', names)).toEqual(['Add Device', 'Add Device…', 'Add Device...'])
  expect(namesMatched('role=button[name="Add Device…"]', names)).toEqual(['Add Device', 'Add Device…', 'Add Device...'])
  expect(namesMatched('role=button[name="add device" i]', names)).toEqual(['Add Device', 'Add Device…', 'Add Device...', 'add device'])
  expect(namesMatched('role=button[name~="device"]', names)).toEqual(names)
  // Characters a regular expression would read stay text.
  expect(namesMatched('role=menuitem[name="Open (a.b)/c"]', ['Open (a.b)/c', 'Open (axb)/c'])).toEqual(['Open (a.b)/c'])
  expect(namesMatched('role=button[name="Say \\"Hi\\""]', ['Say "Hi"'])).toEqual(['Say "Hi"'])
  // A name written as a regular expression, and a CSS attribute named name, are Playwright's as written.
  expect(only('role=button[name=/^Add/]')).toEqual({ kind: 'selector', selector: 'role=button[name=/^Add/]' })
  expect(only('input[name="email"]')).toEqual({ kind: 'selector', selector: 'input[name="email"]' })
})

test('a textbox locator also finds a combobox and an editable element of the same name', () => {
  expect(only('role=textbox[name="Search"]')).toMatchObject({ kind: 'field', combobox: expect.stringMatching(/^role=combobox\[name=\//) })
  expect(namesMatched('role=textbox[name="Search"]', ['Search', 'Search…', 'Research'])).toEqual(['Search', 'Search…'])
  // The composer is an editable element labelled Message, which Playwright gives no role.
  const composer = only('role=textbox[name="Message"]')
  const label = composer.kind === 'field' ? composer.editable?.name : undefined
  expect(['Message', 'Message…', 'Messages'].filter((name) => label?.test(name))).toEqual(['Message', 'Message…'])
  expect(only('role=textbox')).toMatchObject({ kind: 'field', combobox: 'role=combobox', editable: { name: null } })
  // A label cannot answer whether an editable element is disabled.
  expect(only('role=textbox[name="Message"][disabled]')).toMatchObject({ kind: 'field', editable: null })
  // Another role that starts with the same letters is not a textbox.
  expect(only('role=textboxes')).toEqual({ kind: 'selector', selector: 'role=textboxes' })
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

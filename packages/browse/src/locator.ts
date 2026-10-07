// What a command acts on (browse.md § Finding things): a Playwright locator,
// written as Playwright's selectors are (`role=button[name="Reload"]`, CSS,
// chained with `>>`), plus `text=…`, which matches an element's whole text
// as a person reads a label, `text~=…`, which matches a part of it, and
// `label=…` and `testid=…`, which Playwright offers only as methods; or a
// point `x,y` in CSS pixels of the viewport, for a canvas such as the live
// view. A role's quoted name, `[name="Add Device"]`, matches the whole
// accessible name, a trailing ellipsis aside, as a person reads a menu item
// "Add Device…"; `[name~="Device"]` matches a part of it, as `text~=` does.
// A `role=textbox` step also finds a field the page marks as a combobox,
// such as a search field with suggestions, and an editable element with the
// same name, such as the composer, as a person sees a text field either way.
import type { Locator, Page } from 'playwright'

export type Segment =
  | { kind: 'selector', selector: string }
  | { kind: 'text', text: string, exact: boolean }
  | { kind: 'label', text: string, exact: boolean }
  | { kind: 'testid', id: string }
  /**
   * A text field: a textbox, a combobox with the same attributes, or, when
   * `editable` is not null, an editable element whose label `editable.name`
   * matches (any label when null).
   */
  | { kind: 'field', textbox: string, combobox: string, editable: { name: RegExp | null } | null }

export type Target =
  | { kind: 'point', x: number, y: number }
  | { kind: 'locator', text: string, segments: Segment[] }

/** An element a person types into that is no form field, such as a rich text editor. */
const EDITABLE = '[contenteditable]:not([contenteditable="false"])'

/** Splits `text` at ` >> ` outside quotes. */
function chain(text: string): string[] {
  const parts: string[] = []
  let part = ''
  let quote: string | null = null
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]
    if (quote) {
      if (char === '\\') {
        part += char + (text[index + 1] ?? '')
        index += 1
        continue
      }
      if (char === quote) {
        quote = null
      }
      part += char
      continue
    }
    if (char === '"' || char === '\'') {
      quote = char
      part += char
      continue
    }
    if (text.startsWith('>>', index)) {
      parts.push(part.trim())
      part = ''
      index += 1
      continue
    }
    part += char
  }
  parts.push(part.trim())
  return parts
}

/** `"Email"` → exact `Email`; `Email` → `Email`, matched as Playwright matches text. */
function unquote(value: string): { text: string, exact: boolean } {
  const quoted = /^(["'])(.*)\1$/.exec(value)
  return quoted ? { text: quoted[2], exact: true } : { text: value, exact: false }
}

/** A textbox's role selector, `role=textbox` and its attributes, if `part` is one. */
const TEXTBOX = /^role=textbox(?=\[|$)/

/** A role selector's quoted `name` attribute: its operator, quote, value and case flag. */
const QUOTED_NAME = /\[\s*name\s*(~?=)\s*(["'])((?:\\.|(?!\2).)*)\2\s*([iIsS])?\s*\]/g

/** An accessible name's trailing ellipsis, which a person reads past. */
const ELLIPSIS = /(?:…|\.\.\.)$/

/**
 * The name `[name<operator>"<value>"]` matches: the whole name, a trailing
 * ellipsis aside, for `=`; a part of it, in any case, for `~=`, as `text~=`.
 */
function nameMatcher(operator: string, value: string, flag: string | undefined): RegExp {
  if (operator === '~=') {
    return new RegExp(RegExp.escape(value), 'i')
  }
  const whole = value.replace(ELLIPSIS, '')
  return new RegExp(`^${RegExp.escape(whole)}(?:…|\\.\\.\\.)?$`, flag?.toLowerCase() === 'i' ? 'i' : '')
}

/**
 * `part`, a role selector, with each quoted name written as the regular
 * expression `nameMatcher` gives, which Playwright's role selector takes;
 * and the editable fields a textbox step also finds: those whose label
 * matches the quoted name, or null when the selector has an attribute a
 * label cannot answer, such as `[disabled]` or a name given as a regular
 * expression.
 */
function roleSelector(part: string): { selector: string, editable: { name: RegExp | null } | null } {
  let name: RegExp | null = null
  const selector = part.replace(QUOTED_NAME, (_match, operator: string, _quote: string, value: string, flag: string | undefined) => {
    name = nameMatcher(operator, value.replace(/\\(.)/g, '$1'), flag)
    return `[name=/${name.source}/${name.flags}]`
  })
  const others = part.replace(QUOTED_NAME, '').includes('[')
  return { selector, editable: others ? null : { name } }
}

function segment(part: string, text: string): Segment {
  if (part === '') {
    throw new Error(`an empty step in the locator ${text}`)
  }
  // `text~=` before `text=`, which it does not start with but resembles.
  if (part.startsWith('text~=')) {
    return { kind: 'text', text: unquote(part.slice('text~='.length)).text, exact: false }
  }
  if (part.startsWith('text=')) {
    return { kind: 'text', text: unquote(part.slice('text='.length)).text, exact: true }
  }
  if (part.startsWith('label=')) {
    const { text: label, exact } = unquote(part.slice('label='.length))
    return { kind: 'label', text: label, exact }
  }
  if (part.startsWith('testid=')) {
    return { kind: 'testid', id: unquote(part.slice('testid='.length)).text }
  }
  if (!part.startsWith('role=')) {
    return { kind: 'selector', selector: part }
  }
  const role = roleSelector(part)
  if (TEXTBOX.test(role.selector)) {
    return { kind: 'field', textbox: role.selector, combobox: role.selector.replace(TEXTBOX, 'role=combobox'), editable: role.editable }
  }
  return { kind: 'selector', selector: role.selector }
}

export function parseTarget(text: string): Target {
  const point = /^\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*$/.exec(text)
  if (point) {
    return { kind: 'point', x: Number(point[1]), y: Number(point[2]) }
  }
  return { kind: 'locator', text, segments: chain(text).map((part) => segment(part, text)) }
}

/** The elements `segment` names within `scope`. */
function step(scope: Page | Locator, segment: Segment): Locator {
  switch (segment.kind) {
    case 'selector':
      return scope.locator(segment.selector)
    case 'text':
      // Playwright's exact text is the element's whole text, case and all, its surrounding spaces trimmed.
      return scope.getByText(segment.text, { exact: segment.exact })
    case 'label':
      return scope.getByLabel(segment.text, { exact: segment.exact })
    case 'testid':
      return scope.getByTestId(segment.id)
    case 'field': {
      const field = scope.locator(segment.textbox).or(scope.locator(segment.combobox))
      if (segment.editable === null) {
        return field
      }
      // Playwright gives an editable element no role; its label names it as a role's name does.
      const editable = scope.locator(EDITABLE)
      const named = segment.editable.name === null ? editable : editable.and(scope.getByLabel(segment.editable.name))
      return field.or(named)
    }
  }
}

/** The Playwright locator of `segments`, which `parseTarget` never leaves empty, in `page`. */
export function locate(page: Page, segments: Segment[]): Locator {
  const [first, ...rest] = segments
  return rest.reduce(step, step(page, first))
}

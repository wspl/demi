// What a command acts on (product-checks.md § Finding things): a Playwright
// locator, written as Playwright's selectors are (`role=button[name="Reload"]`,
// `text=…`, CSS, chained with `>>`), plus `label=…` and `testid=…`, which
// Playwright offers only as methods; or a point `x,y` in CSS pixels of the
// viewport, for a canvas such as the live view.
import type { Locator, Page } from 'playwright'

export type Segment =
  | { kind: 'selector', selector: string }
  | { kind: 'label', text: string, exact: boolean }
  | { kind: 'testid', id: string }

export type Target =
  | { kind: 'point', x: number, y: number }
  | { kind: 'locator', text: string, segments: Segment[] }

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

export function parseTarget(text: string): Target {
  const point = /^\s*(-?\d+(?:\.\d+)?)\s*,\s*(-?\d+(?:\.\d+)?)\s*$/.exec(text)
  if (point) {
    return { kind: 'point', x: Number(point[1]), y: Number(point[2]) }
  }
  const segments = chain(text).map((part): Segment => {
    if (part === '') {
      throw new Error(`an empty step in the locator ${text}`)
    }
    if (part.startsWith('label=')) {
      const { text: label, exact } = unquote(part.slice('label='.length))
      return { kind: 'label', text: label, exact }
    }
    if (part.startsWith('testid=')) {
      return { kind: 'testid', id: unquote(part.slice('testid='.length)).text }
    }
    return { kind: 'selector', selector: part }
  })
  return { kind: 'locator', text, segments }
}

/** The elements `segment` names within `scope`. */
function step(scope: Page | Locator, segment: Segment): Locator {
  switch (segment.kind) {
    case 'selector':
      return scope.locator(segment.selector)
    case 'label':
      return scope.getByLabel(segment.text, { exact: segment.exact })
    case 'testid':
      return scope.getByTestId(segment.id)
  }
}

/** The Playwright locator of `segments`, which `parseTarget` never leaves empty, in `page`. */
export function locate(page: Page, segments: Segment[]): Locator {
  const [first, ...rest] = segments
  return rest.reduce(step, step(page, first))
}

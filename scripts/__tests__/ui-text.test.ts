// The UI text check (`scripts/ui-text-check.ts`) against the baseline of the
// violations not yet fixed: a new violation fails, and so does a baseline
// line nothing reports any more, so the baseline only shrinks.
import { expect, test } from 'bun:test'
import { baselineLine, checkUiText, describe, readBaseline } from '../ui-text-check'

// About 5 seconds: following a literal into the prop it fills needs the
// type-checked program of every Vue package, templates included, and no
// smaller program sees a gallery specimen or a plugin fill a web-ui prop.
test('UI text keeps macOS capitalization, except the violations the baseline lists', () => {
  const violations = checkUiText().checked.filter((text) => text.problems.length > 0)
  const baseline = readBaseline()
  const reported = new Set(violations.map(baselineLine))
  const added = violations.filter((text) => !baseline.has(baselineLine(text))).map(describe)
  const fixed = [...baseline].filter((line) => !reported.has(line))
  expect({ added, fixed }).toEqual({ added: [], fixed: [] })
}, 30_000)

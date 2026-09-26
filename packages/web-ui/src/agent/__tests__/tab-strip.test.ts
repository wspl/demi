import { expect, test } from 'bun:test'
import { revealScroll } from '../tab-strip'

test('the active tab is revealed clear of the fades, and a tab too wide for them settles on its start', () => {
  const view = { scrollLeft: 0, clientWidth: 300, scrollWidth: 900 }
  // Whole and clear of both fades: nothing moves.
  expect(revealScroll(view, { left: 40, right: 140 }, 24)).toBeNull()
  // Past the right edge: just far enough to clear the fade.
  expect(revealScroll(view, { left: 400, right: 500 }, 24)).toBe(224)
  // Behind the left edge.
  expect(revealScroll({ ...view, scrollLeft: 500 }, { left: 400, right: 500 }, 24)).toBe(376)
  // The last tab: the strip's end is as far as it goes, and once there nothing moves.
  expect(revealScroll(view, { left: 800, right: 900 }, 24)).toBe(600)
  expect(revealScroll({ ...view, scrollLeft: 600 }, { left: 800, right: 900 }, 24)).toBeNull()
  // A strip narrower than the tab with its fades: the start wins from either side, then it rests.
  const narrow = { scrollLeft: 307, clientWidth: 129, scrollWidth: 436 }
  const last = { left: 323, right: 411 }
  expect(revealScroll(narrow, last, 24)).toBe(299)
  expect(revealScroll({ ...narrow, scrollLeft: 299 }, last, 24)).toBeNull()
})

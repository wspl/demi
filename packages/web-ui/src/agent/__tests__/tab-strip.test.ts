import { expect, test } from 'bun:test'
import { cutMarkCover, fadeRoom, revealScroll, tabsChanged, type StripMutation } from '../tab-strip'

test('the selected tab is revealed whole and clear of the fades, and where they do not fit the fades give way', () => {
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
  // A view narrower than the tab with its fades: the tab shows whole, its end included, and the
  // fades shrink to the room beside it, so none lies over it.
  const narrow = { scrollLeft: 340, clientWidth: 129, scrollWidth: 600 }
  const middle = { left: 323, right: 411 }
  expect(revealScroll(narrow, middle, 24)).toBe(323)
  expect(revealScroll({ ...narrow, scrollLeft: 290 }, middle, 24)).toBeNull()
  expect(fadeRoom(129, { left: middle.left - 290, right: middle.right - 290 })).toEqual({ before: 33, after: 8 })
  // A tab running off an edge leaves that fade no room.
  expect(fadeRoom(129, { left: -4, right: 84 })).toEqual({ before: 0, after: 45 })
})

test('a scrolled edge covers a tab mark it cuts, and only that one', () => {
  // Marks at 6–22 of tabs starting at -16, 120 and 290, in a view 300 wide.
  const marks = [{ left: -10, right: 6 }, { left: 126, right: 142 }, { left: 296, right: 312 }]
  // The start edge cuts the first mark: the fade stays solid past its end.
  expect(cutMarkCover(0, 'start', marks)).toBe(6)
  // The end edge cuts the last one: solid from its start to the edge.
  expect(cutMarkCover(300, 'end', marks)).toBe(4)
  // An edge between marks, or on a mark's own edge, cuts none.
  expect(cutMarkCover(100, 'end', marks)).toBe(0)
  expect(cutMarkCover(296, 'end', marks)).toBe(0)
  // Fractional layout rounds up, so no sliver of the mark shows.
  expect(cutMarkCover(0, 'start', [{ left: -3.5, right: 12.25 }])).toBe(13)
})

test('only a tab becoming active, coming or going moves the view, not a render of the strip', () => {
  const strip = { name: 'strip' }
  const tab = { name: 'tab' }
  const copy = { name: 'shallow copy' }
  const isTab = (node: { name: string }): boolean => node.name !== 'text'
  const change = (
    target: unknown,
    added: { name: string }[],
    removed: { name: string }[],
  ): StripMutation<{ name: string }> => ({ type: 'childList', target, addedNodes: added, removedNodes: removed })
  // A new tab, a closed tab, a new selection.
  expect(tabsChanged(strip, [change(strip, [tab], [])], isTab)).toBe(true)
  expect(tabsChanged(strip, [change(strip, [], [tab])], isTab)).toBe(true)
  expect(tabsChanged(strip, [{ type: 'attributes', target: tab, addedNodes: [], removedNodes: [] }], isTab)).toBe(true)
  // The transition group's shallow copy of a tab, in and out within one render.
  expect(tabsChanged(strip, [change(strip, [copy], []), change(strip, [], [copy])], isTab)).toBe(false)
  // A tab taken out and put back, and the text anchors a render moves.
  expect(tabsChanged(strip, [change(strip, [], [tab]), change(strip, [tab], [])], isTab)).toBe(false)
  expect(tabsChanged(strip, [change(strip, [{ name: 'text' }], [])], isTab)).toBe(false)
  // A page's title changing inside its tab.
  expect(tabsChanged(strip, [change(tab, [{ name: 'title' }], [{ name: 'title' }])], isTab)).toBe(false)
})

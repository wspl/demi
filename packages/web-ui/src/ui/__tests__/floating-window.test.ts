import { describe, expect, test } from 'bun:test'
import {
  clampWindowRect,
  escapeClosesWindow,
  moveWindowRect,
  resizeWindowRect,
  windowCentre,
  windowRectAround,
  windowSizeFor,
  WINDOW_TITLE_BAR_PX,
} from '../floating-window'

// Pure geometry and key rules: no DOM, a few milliseconds in all.

const PAGE = { width: 1440, height: 900 }

describe('a window stays on the page', () => {
  test.each([
    ['a box on the page stays as it is', { x: 100, y: 80, width: 600, height: 400 }, { x: 100, y: 80, width: 600, height: 400 }],
    ['a box smaller than 320 × 240 grows to it', { x: 100, y: 80, width: 100, height: 50 }, { x: 100, y: 80, width: 320, height: 240 }],
    ['a box larger than the page shrinks to it', { x: 0, y: 0, width: 2000, height: 1200 }, { x: 0, y: 0, width: 1440, height: 900 }],
    ['a box dragged past the left and top stops at them', { x: -300, y: -50, width: 600, height: 400 }, { x: 0, y: 0, width: 600, height: 400 }],
    ['a box dragged past the right stays wholly on the page', { x: 1200, y: 80, width: 600, height: 400 }, { x: 840, y: 80, width: 600, height: 400 }],
    ['a box dragged down keeps its title bar on the page and hangs below it', { x: 100, y: 2000, width: 600, height: 400 }, { x: 100, y: 900 - WINDOW_TITLE_BAR_PX, width: 600, height: 400 }],
  ])('%s', (_, rect, expected) => {
    expect(clampWindowRect(rect, PAGE)).toEqual(expected)
  })

  test('a browser window that shrinks moves and shrinks the window with it', () => {
    const rect = { x: 900, y: 500, width: 500, height: 350 }
    expect(clampWindowRect(rect, { width: 800, height: 600 })).toEqual({ x: 300, y: 500, width: 500, height: 350 })
    // Its title bar stays on the page; the rest hangs below, as one the user dragged down does.
    expect(clampWindowRect(rect, { width: 400, height: 300 })).toEqual({ x: 0, y: 300 - WINDOW_TITLE_BAR_PX, width: 400, height: 300 })
  })

  test('a page smaller than the minimum holds a window as large as itself', () => {
    expect(clampWindowRect({ x: 10, y: 10, width: 240, height: 180 }, { width: 200, height: 150 }))
      .toEqual({ x: 0, y: 10, width: 200, height: 150 })
  })

  test('a title bar drag moves the box by the pointer and stops at the page', () => {
    const start = { x: 100, y: 100, width: 600, height: 400 }
    expect(moveWindowRect(start, 40, -30, PAGE)).toEqual({ x: 140, y: 70, width: 600, height: 400 })
    expect(moveWindowRect(start, -500, -500, PAGE)).toEqual({ x: 0, y: 0, width: 600, height: 400 })
  })
})

describe('an edge or a corner resizes the window, the opposite edges staying put', () => {
  const start = { x: 400, y: 200, width: 600, height: 400 }
  test.each([
    ['the right edge', { right: true }, 50, 99, { x: 400, y: 200, width: 650, height: 400 }],
    ['the left edge', { left: true }, -50, 99, { x: 350, y: 200, width: 650, height: 400 }],
    ['the bottom edge', { bottom: true }, 99, 60, { x: 400, y: 200, width: 600, height: 460 }],
    ['the top edge', { top: true }, 99, -60, { x: 400, y: 140, width: 600, height: 460 }],
    ['the top-left corner', { top: true, left: true }, 30, 20, { x: 430, y: 220, width: 570, height: 380 }],
    ['the bottom-right corner', { bottom: true, right: true }, 30, 20, { x: 400, y: 200, width: 630, height: 420 }],
    ['the left edge stops at the minimum, its right edge in place', { left: true }, 500, 0, { x: 680, y: 200, width: 320, height: 400 }],
    ['the top edge stops at the minimum, its bottom edge in place', { top: true }, 0, 500, { x: 400, y: 360, width: 600, height: 240 }],
    ['the left edge stops at the page', { left: true }, -900, 0, { x: 0, y: 200, width: 1000, height: 400 }],
    ['the right edge stops at the page', { right: true }, 900, 0, { x: 400, y: 200, width: 1040, height: 400 }],
    ['the bottom edge stops at the page', { bottom: true }, 0, 900, { x: 400, y: 200, width: 600, height: 700 }],
  ])('%s', (_, edges, dx, dy, expected) => {
    expect(resizeWindowRect(start, edges, dx, dy, PAGE)).toEqual(expected)
  })
})

describe('a window opens at its content\'s actual size in device pixels', () => {
  // The page is 1440 × 900: the window may take 864 of its width and 630 of its height, the title bar included.
  test.each([
    ['a screenshot at one device pixel per page pixel', { width: 400, height: 300 }, 1, { width: 400, height: 300 + WINDOW_TITLE_BAR_PX }],
    ['a retina screenshot, 2880 × 1800 on two, is 1440 × 900 and scales down to the page\'s allowance', { width: 2880, height: 1800 }, 2, { width: 864, height: 540 + WINDOW_TITLE_BAR_PX }],
    ['the same picture on one device pixel scales down just the same', { width: 2880, height: 1800 }, 1, { width: 864, height: 540 + WINDOW_TITLE_BAR_PX }],
    ['a retina picture that fits once halved opens at half its pixels', { width: 800, height: 600 }, 2, { width: 400, height: 300 + WINDOW_TITLE_BAR_PX }],
    ['a wide picture is held by the width, 1280 × 720', { width: 1280, height: 720 }, 1, { width: 864, height: 486 + WINDOW_TITLE_BAR_PX }],
    ['a tall page is held by the height and keeps the minimum width', { width: 360, height: 2400 }, 1, { width: 320, height: 630 }],
    ['a tiny icon sits in a window of the minimum size, never enlarged', { width: 40, height: 40 }, 1, { width: 320, height: 240 }],
    ['a tiny retina icon too', { width: 80, height: 80 }, 2, { width: 320, height: 240 }],
    ['a picture of unknown size takes the whole allowance', null, 2, { width: 864, height: 630 }],
  ])('%s', (_, content, pixelRatio, expected) => {
    expect(windowSizeFor(content, pixelRatio, PAGE)).toEqual(expected)
  })

  test('on a page smaller than the minimum the window is as large as the page', () => {
    expect(windowSizeFor({ width: 40, height: 40 }, 1, { width: 300, height: 200 })).toEqual({ width: 300, height: 200 })
  })

  test('a window centres on a point and stays on the page', () => {
    const size = { width: 400, height: 336 }
    expect(windowRectAround(size, { x: 870, y: 450 }, PAGE)).toEqual({ x: 670, y: 282, width: 400, height: 336 })
    expect(windowRectAround(size, { x: 1400, y: 20 }, PAGE)).toEqual({ x: 1040, y: 0, width: 400, height: 336 })
  })

  test('the centre of a box is the middle of its sides', () => {
    expect(windowCentre({ x: 670, y: 282, width: 400, height: 336 })).toEqual({ x: 870, y: 450 })
  })
})

describe('Escape closes a window', () => {
  /** An element the focus is on, which the given selectors in a selector list match. */
  function focusOn(own: string) {
    return { closest: (selectors: string) => (selectors.split(',').some((selector) => selector.trim() === own) ? {} : null) }
  }
  const key = { defaultPrevented: false, isComposing: false, target: focusOn('button') }

  test('with the focus in the window or on a control of the page', () => {
    expect(escapeClosesWindow(key, false)).toBe(true)
    expect(escapeClosesWindow({ ...key, target: null }, false)).toBe(true)
  })

  test.each([
    ['a text field', 'textarea'],
    ['the composer or an editor', '[contenteditable=true]'],
    ['a combo box', '[role=combobox]'],
  ])('not with the focus in %s', (_, own) => {
    expect(escapeClosesWindow({ ...key, target: focusOn(own) }, false)).toBe(false)
  })

  test('not while a menu or a dialog is open', () => {
    expect(escapeClosesWindow(key, true)).toBe(false)
  })

  test('not when something used the key first, or an input method composes', () => {
    expect(escapeClosesWindow({ ...key, defaultPrevented: true }, false)).toBe(false)
    expect(escapeClosesWindow({ ...key, isComposing: true }, false)).toBe(false)
  })
})

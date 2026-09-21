import { expect, test } from 'bun:test'
import {
  afterEnterTab,
  afterLeaveTab,
  beforeEnterTab,
  beforeLeaveTab,
  enterTab,
  leaveTab,
  revealScroll,
} from '../tab-strip'

function tab(
  displayed: number,
  gap: string | null = null,
): HTMLElement & {
  unlocked: number
} {
  const style = {} as CSSStyleDeclaration
  const state = { unlocked: displayed }
  const el = {
    style,
    scrollWidth: displayed as number,
    unlocked: displayed,
    parentElement: gap === null ? null : {},
    classList: { contains: () => false },
    getBoundingClientRect: () => {
      if (style.width !== '' && style.width !== 'auto') {
        const locked = Number.parseFloat(style.width)
        if (Number.isFinite(locked)) {
          return { width: locked }
        }
      }
      return { width: state.unlocked }
    },
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  }
  Object.defineProperty(el, 'unlocked', {
    get: () => state.unlocked,
    set: (value: number) => {
      state.unlocked = value
    },
  })
  return el as unknown as HTMLElement & { unlocked: number }
}

test('leave locks sibling widths so the rest of the strip does not reflow under the closing tab', () => {
  const sibling = tab(120)
  const el = tab(148, '2px')
  const parent = { children: [sibling, el] as HTMLElement[] }
  Object.assign(el, { parentElement: parent })
  Object.assign(sibling, { parentElement: parent })
  const previous = globalThis.getComputedStyle
  globalThis.getComputedStyle = () => ({ columnGap: '2px' }) as CSSStyleDeclaration
  try {
    beforeLeaveTab(el)
    expect(sibling.style.width).toBe('120px')
    expect(sibling.style.flexShrink).toBe('0')
    leaveTab(el)
    parent.children = [sibling]
    afterLeaveTab(el)
    expect(sibling.style.width).toBe('')
    expect(sibling.style.flexShrink).toBe('')
  } finally {
    globalThis.getComputedStyle = previous
  }
})

test('after leave, squeezed tabs ease to the new width instead of snapping', () => {
  const sibling = tab(118)
  const el = tab(118)
  const parent = {
    children: [sibling, el],
    offsetWidth: 0,
  }
  Object.assign(el, { parentElement: parent })
  Object.assign(sibling, { parentElement: parent })
  const previous = globalThis.getComputedStyle
  globalThis.getComputedStyle = () => ({ columnGap: '0px' }) as CSSStyleDeclaration
  try {
    beforeLeaveTab(el)
    leaveTab(el)
    parent.children = [sibling]
    sibling.unlocked = 160
    afterLeaveTab(el)
    expect(sibling.style.width).toBe('160px')
    expect(sibling.style.transition).toContain('width')
    expect(sibling.style.flexShrink).toBe('0')
  } finally {
    globalThis.getComputedStyle = previous
  }
})

test('leave locks the current width then collapses it and eats the strip gap', () => {
  const el = tab(148, '2px')
  const previous = globalThis.getComputedStyle
  globalThis.getComputedStyle = () => ({ columnGap: '2px' }) as CSSStyleDeclaration
  try {
    beforeLeaveTab(el)
    expect(el.style.width).toBe('148px')
    expect(el.style.flexShrink).toBe('0')
    leaveTab(el)
    expect(el.style.width).toBe('0')
    expect(el.style.opacity).toBe('0')
    expect(el.style.marginRight).toBe('-2px')
  } finally {
    globalThis.getComputedStyle = previous
  }
})

test('enter grows from zero to the open width, then after-enter clears the lock', () => {
  const el = tab(160)
  beforeEnterTab(el)
  expect(el.style.opacity).toBe('0')
  enterTab(el)
  expect(el.style.width).toBe('160px')
  afterEnterTab(el)
  expect(el.style.width).toBe('')
  expect(el.style.flexShrink).toBe('')
})

test('enter uses the open width, not the collapsed icon width', () => {
  const el = tab(160)
  Object.defineProperty(el, 'scrollWidth', { value: 28 })
  beforeEnterTab(el)
  enterTab(el)
  expect(el.style.width).toBe('160px')
})

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

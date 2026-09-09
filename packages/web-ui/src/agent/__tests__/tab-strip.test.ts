import { expect, test } from 'bun:test'
import {
  afterEnterTab,
  afterLeaveTab,
  beforeEnterTab,
  beforeLeaveTab,
  enterTab,
  leaveTab,
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
